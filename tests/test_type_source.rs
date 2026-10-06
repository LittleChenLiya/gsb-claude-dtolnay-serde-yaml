use serde::de::{self, Deserialize};
use serde_derive::Deserialize;
use serde_yaml::{Deserializer, NodeKind};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

// A field that expects an integer but meets a sequence is reported on the
// sequence node itself, with the expected type recorded explicitly.
#[test]
fn test_field_expects_integer_finds_sequence() {
    #[derive(Deserialize, Debug)]
    struct S {
        #[allow(dead_code)]
        x: i64,
    }

    let yaml = "x: [1, 2]\n";
    let err = serde_yaml::from_str::<S>(yaml).unwrap_err();
    assert_eq!(
        "x: invalid type: sequence, expected i64 at line 1 column 4",
        err.to_string()
    );

    let location = err.location().unwrap();
    assert_eq!(location.line(), 1);
    assert_eq!(location.column(), 4);

    let source = err.type_source().unwrap();
    assert_eq!(source.expected(), "i64");
    assert_eq!(source.found(), NodeKind::Sequence);
    assert_eq!(source.location().unwrap().line(), 1);
    assert_eq!(source.location().unwrap().column(), 4);
    assert!(source.alias().is_none());
}

// An error inside a node reached through an alias points at both the alias
// reference and the anchored definition.
#[test]
fn test_alias_error_reports_reference_and_definition() {
    #[derive(Deserialize, Debug)]
    struct S {
        #[allow(dead_code)]
        a: Vec<i64>,
        #[allow(dead_code)]
        b: i64,
    }

    let yaml = "a: &x [1, 2]\nb: *x\n";
    let err = serde_yaml::from_str::<S>(yaml).unwrap_err();
    assert_eq!(
        "b: invalid type: sequence, expected i64 at line 1 column 4 (referenced by alias at line 2 column 4)",
        err.to_string()
    );

    // The primary location is the anchored definition of `*x`.
    let location = err.location().unwrap();
    assert_eq!(location.line(), 1);
    assert_eq!(location.column(), 4);

    let source = err.type_source().unwrap();
    assert_eq!(source.expected(), "i64");
    assert_eq!(source.found(), NodeKind::Sequence);
    let alias = source.alias().unwrap();
    assert_eq!(alias.reference().line(), 2);
    assert_eq!(alias.reference().column(), 4);
    assert_eq!(alias.definition().line(), 1);
    assert_eq!(alias.definition().column(), 4);
}

// The kind of node that caused a mismatch is recorded for scalars, mappings
// and explicitly tagged nodes as well.
#[test]
fn test_node_kinds() {
    let err = serde_yaml::from_str::<i64>("hello\n").unwrap_err();
    let source = err.type_source().unwrap();
    assert_eq!(source.expected(), "i64");
    assert_eq!(source.found(), NodeKind::Scalar);
    assert_eq!(source.location().unwrap().line(), 1);

    let err = serde_yaml::from_str::<i64>("k: v\n").unwrap_err();
    let source = err.type_source().unwrap();
    assert_eq!(source.found(), NodeKind::Mapping);

    let err = serde_yaml::from_str::<i64>("!!str hello\n").unwrap_err();
    let source = err.type_source().unwrap();
    assert_eq!(source.found(), NodeKind::Tagged);
}

// The path and position are computed during the one and only deserialization
// pass; Deserialize is never run a second time to guess them.
#[test]
fn test_user_deserialize_runs_once() {
    static RUNS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct Once;

    impl<'de> Deserialize<'de> for Once {
        fn deserialize<D>(deserializer: D) -> Result<Once, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            RUNS.fetch_add(1, Ordering::SeqCst);
            // Expect an integer where the input has a sequence.
            i64::deserialize(deserializer)?;
            Ok(Once)
        }
    }

    RUNS.store(0, Ordering::SeqCst);
    let err = serde_yaml::from_str::<Once>("[1, 2]\n").unwrap_err();
    assert_eq!(RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(err.type_source().unwrap().found(), NodeKind::Sequence);
    assert_eq!(err.location().unwrap().line(), 1);
}

// Errors produced by the user's own Deserialize implementation keep their
// original message (the cause) and are not reclassified as type mismatches.
#[test]
fn test_user_error_cause_preserved() {
    #[derive(Debug)]
    struct Custom;

    impl<'de> Deserialize<'de> for Custom {
        fn deserialize<D>(deserializer: D) -> Result<Custom, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct V;
            impl<'de> de::Visitor<'de> for V {
                type Value = Custom;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("anything")
                }

                fn visit_map<A>(self, mut map: A) -> Result<Custom, A::Error>
                where
                    A: de::MapAccess<'de>,
                {
                    while map
                        .next_entry::<de::IgnoredAny, de::IgnoredAny>()?
                        .is_some()
                    {}
                    Err(de::Error::custom("my custom cause"))
                }
            }
            deserializer.deserialize_map(V)
        }
    }

    let err = serde_yaml::from_str::<Custom>("x: 1\n").unwrap_err();
    // The user's message is kept verbatim as the error's cause.
    assert!(err.to_string().starts_with("my custom cause"));
    assert!(err.type_source().is_none());
}

// Each document of a multi-document input reports positions within itself;
// positions do not leak from one document into another.
#[test]
fn test_multi_document_positions_do_not_cross() {
    #[derive(Deserialize, Debug)]
    struct S {
        #[allow(dead_code)]
        x: i64,
    }

    let yaml = "---\nx: 1\n---\nx: [1]\n";
    let mut de = Deserializer::from_str(yaml);

    let first = de.next().unwrap();
    assert!(S::deserialize(first).is_ok());

    let second = de.next().unwrap();
    let err = S::deserialize(second).unwrap_err();
    assert_eq!(
        "x: invalid type: sequence, expected i64 at line 4 column 4",
        err.to_string()
    );
    assert_eq!(err.location().unwrap().line(), 4);
    assert_eq!(err.location().unwrap().column(), 4);

    // Same mismatch in the first document instead: position is document 1's.
    let yaml = "x: [1]\n---\nx: 2\n";
    let mut de = Deserializer::from_str(yaml);

    let first = de.next().unwrap();
    let err = S::deserialize(first).unwrap_err();
    assert_eq!(
        "x: invalid type: sequence, expected i64 at line 1 column 4",
        err.to_string()
    );
    assert_eq!(err.location().unwrap().line(), 1);

    let second = de.next().unwrap();
    assert!(S::deserialize(second).is_ok());
}

// Errors that have no input position do not fabricate one (no "line 0").
#[test]
fn test_unknown_position_not_fabricated() {
    let yaml = "---\n0\n---\n1\n";
    let err = serde_yaml::from_str::<usize>(yaml).unwrap_err();
    assert_eq!(
        "deserializing from YAML containing more than one document is not supported",
        err.to_string()
    );
    assert!(err.location().is_none());
    assert!(err.type_source().is_none());
    assert!(!err.to_string().contains("line 0"));
}

// Successful results are unaffected by the error reporting.
#[test]
fn test_normal_results_compatible() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct S {
        a: Vec<i64>,
        b: i64,
    }

    let yaml = "a: &x [1, 2]\nb: 3\n";
    let s = serde_yaml::from_str::<S>(yaml).unwrap();
    assert_eq!(
        s,
        S {
            a: vec![1, 2],
            b: 3,
        }
    );

    let value: serde_yaml::Value = serde_yaml::from_str("a: &x 1\nb: *x\n").unwrap();
    assert_eq!(value.get("b"), value.get("a"));
}
