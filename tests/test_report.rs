use serde::de::{self, Visitor};
use serde::Deserialize;
use serde_derive::Deserialize;
use serde_yaml::report::{NodeKind, TypeReporter};
use serde_yaml::Deserializer;
use std::cell::Cell;
use std::fmt;

#[derive(Deserialize, Debug)]
struct Config {
    port: u16,
}

#[test]
fn test_sequence_where_integer_expected() {
    // A field that expects an integer but meets a sequence is attributed to
    // the sequence node itself.
    let yaml = "port:\n  - 1\n  - 2\n";
    let reporter = TypeReporter::new();
    let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
    let err = Config::deserialize(de).unwrap_err();
    assert_eq!(
        err.to_string(),
        "port: invalid type: sequence, expected u16 at line 2 column 3",
    );
    let location = err.location().unwrap();
    assert_eq!(location.line(), 2);
    assert_eq!(location.column(), 3);

    let report = reporter.report();
    let entry = report.entries().last().unwrap();
    assert_eq!(entry.expected(), "u16");
    assert_eq!(entry.node(), NodeKind::Sequence);
    assert_eq!(entry.location(), Some(location));
    assert_eq!(entry.alias(), None);
}

#[test]
fn test_alias_error_definition_and_reference() {
    // An error behind an alias points at both the reference and the
    // definition.
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Aliased {
        a: Vec<u16>,
        b: u16,
    }

    let yaml = "a: &x [1, 2]\nb: *x\n";
    let reporter = TypeReporter::new();
    let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
    let err = Aliased::deserialize(de).unwrap_err();
    assert_eq!(
        err.to_string(),
        "b: invalid type: sequence, expected u16 at line 1 column 4 \
         (alias defined at line 1 column 4, referenced at line 2 column 4)",
    );

    let report = reporter.report();
    let entry = report
        .entries()
        .iter()
        .find(|entry| entry.alias().is_some())
        .unwrap();
    assert_eq!(entry.expected(), "u16");
    assert_eq!(entry.node(), NodeKind::Sequence);
    let alias = entry.alias().unwrap();
    assert_eq!(
        (alias.definition().line(), alias.definition().column()),
        (1, 4),
    );
    assert_eq!(
        (alias.reference().line(), alias.reference().column()),
        (2, 4),
    );
}

thread_local! {
    static CALLS: Cell<usize> = Cell::new(0);
}

#[derive(Debug)]
struct Counted;

impl<'de> Deserialize<'de> for Counted {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        CALLS.with(|calls| calls.set(calls.get() + 1));
        deserializer.deserialize_any(CountedVisitor)
    }
}

struct CountedVisitor;

impl<'de> Visitor<'de> for CountedVisitor {
    type Value = Counted;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a counted value")
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Counted, E> {
        Ok(Counted)
    }
}

#[test]
fn test_user_deserialize_runs_once() {
    // Positions and the type-source report are produced during the single
    // normal pass; the user's Deserialize is never run a second time to
    // guess a path.
    CALLS.with(|calls| calls.set(0));
    Counted::deserialize(Deserializer::from_str("107\n")).unwrap();
    CALLS.with(|calls| assert_eq!(calls.get(), 1));

    CALLS.with(|calls| calls.set(0));
    Counted::deserialize(Deserializer::from_str("nope\n")).unwrap_err();
    CALLS.with(|calls| assert_eq!(calls.get(), 1));
}

#[derive(Debug)]
struct Failing;

impl<'de> Deserialize<'de> for Failing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        CALLS.with(|calls| calls.set(calls.get() + 1));
        deserializer.deserialize_any(FailingVisitor)
    }
}

struct FailingVisitor;

impl<'de> Visitor<'de> for FailingVisitor {
    type Value = Failing;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a failing value")
    }

    fn visit_map<A>(self, _map: A) -> Result<Failing, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        Err(de::Error::custom("user's own cause"))
    }
}

#[test]
fn test_user_error_cause_preserved() {
    CALLS.with(|calls| calls.set(0));
    let yaml = "---\nk: v\n";
    let reporter = TypeReporter::new();
    let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
    let err = Failing::deserialize(de).unwrap_err();
    CALLS.with(|calls| assert_eq!(calls.get(), 1));

    // The user's own error message is kept, with the position filled in.
    assert_eq!(err.to_string(), "user's own cause at line 2 column 1");
    let location = err.location().unwrap();
    assert_eq!((location.line(), location.column()), (2, 1));
}

#[test]
fn test_multi_document_positions_not_mixed() {
    let yaml = "---\nport: 1\n---\nport:\n  - 1\n";
    let reporter = TypeReporter::new();
    let mut documents = Deserializer::from_str(yaml).with_type_reporter(&reporter);

    let first = documents.next().unwrap();
    let config = Config::deserialize(first).unwrap();
    assert_eq!(config.port, 1);

    // Positions recorded for the first document stay in the first document.
    let first_report = reporter.report();
    assert!(!first_report.is_empty());
    for entry in first_report.entries() {
        let location = entry.location().unwrap();
        assert!(location.line() <= 2);
    }

    reporter.clear();

    let second = documents.next().unwrap();
    let err = Config::deserialize(second).unwrap_err();
    assert_eq!(
        err.to_string(),
        "port: invalid type: sequence, expected u16 at line 5 column 3",
    );
    assert_eq!(err.location().unwrap().line(), 5);

    // Entries recorded for the second document carry its own positions.
    let second_report = reporter.report();
    assert!(!second_report.is_empty());
    for entry in second_report.entries() {
        let location = entry.location().unwrap();
        assert!(location.line() >= 4);
    }
    let entry = second_report.entries().last().unwrap();
    assert_eq!(entry.node(), NodeKind::Sequence);
    assert_eq!(entry.location().unwrap().line(), 5);
}

#[test]
fn test_tag_recorded() {
    let yaml = "---\n!!int 107\n";
    let reporter = TypeReporter::new();
    let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
    let value = u64::deserialize(de).unwrap();
    assert_eq!(value, 107);

    let report = reporter.report();
    assert!(report
        .entries()
        .iter()
        .any(|entry| entry.node() == NodeKind::Scalar));
    let tag = report
        .entries()
        .iter()
        .find(|entry| entry.node() == NodeKind::Tag)
        .unwrap();
    assert_eq!(tag.expected(), "u64");
    assert_eq!(tag.location().unwrap().line(), 2);
}

#[test]
fn test_unknown_position_not_faked() {
    // The root node of this input sits at the parser's zero mark, which
    // carries no position information. It is reported as unknown, never as
    // a fabricated line 0.
    let yaml = "[1, 2]\n";
    let reporter = TypeReporter::new();
    let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
    let value = Vec::<u16>::deserialize(de).unwrap();
    assert_eq!(value, [1, 2]);

    let report = reporter.report();
    let root = report.entries().first().unwrap();
    assert_eq!(root.node(), NodeKind::Sequence);
    assert_eq!(root.location(), None);
    for entry in report.entries() {
        if let Some(location) = entry.location() {
            assert!(location.line() >= 1);
        }
    }
}

#[test]
fn test_report_on_success_and_error() {
    // The reporter is compatible with both the Ok and the Err outcome.
    let reporter = TypeReporter::new();
    let de = Deserializer::from_str("port: 8080\n").with_type_reporter(&reporter);
    let config = Config::deserialize(de).unwrap();
    assert_eq!(config.port, 8080);
    assert!(!reporter.report().is_empty());

    let reporter = TypeReporter::new();
    let de = Deserializer::from_str("port: nope\n").with_type_reporter(&reporter);
    assert!(Config::deserialize(de).is_err());
    assert!(!reporter.report().is_empty());
}
