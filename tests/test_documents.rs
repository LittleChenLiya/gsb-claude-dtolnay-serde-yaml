use serde::de::Deserialize as _;
use serde_derive::Deserialize;
use serde_yaml::Value;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, PartialEq)]
struct Person {
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Point {
    x: i64,
    y: i64,
}

#[test]
fn two_documents_distinct_types_each_callback_once() {
    let input = "---\nname: Ada\n---\nx: 1\ny: 2\n";

    let mut people = Vec::new();
    let mut points = Vec::new();
    let mut people_calls = 0usize;
    let mut points_calls = 0usize;

    serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                people_calls += 1;
                people.push(Person::deserialize(de)?);
                Ok(())
            },
            &mut |de| {
                points_calls += 1;
                points.push(Point::deserialize(de)?);
                Ok(())
            },
        ],
    )
    .unwrap();

    assert_eq!(
        people,
        vec![Person {
            name: "Ada".to_owned()
        }]
    );
    assert_eq!(points, vec![Point { x: 1, y: 2 }]);
    assert_eq!(people_calls, 1);
    assert_eq!(points_calls, 1);
}

#[test]
fn second_document_type_error_is_attributed_to_second_document() {
    // First document is a valid Point; second is a map that cannot
    // deserialize into Point (missing required fields).
    let input = "x: 7\ny: 8\n---\nname: Grace\n";
    let second_start = input.find("name: Grace").unwrap();

    let mut first_calls = 0usize;
    let mut second_calls = 0usize;

    let err = serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                first_calls += 1;
                Point::deserialize(de).map(drop)
            },
            &mut |de| {
                second_calls += 1;
                Point::deserialize(de).map(drop)
            },
        ],
    )
    .unwrap_err();

    let doc = err.document().expect("error attributed to a document");
    assert_eq!(doc.document(), 2);
    // The retained input range must cover the second document.
    assert!(
        doc.start() <= second_start && doc.end() > second_start,
        "range {}..{} should contain second document at {}",
        doc.start(),
        doc.end(),
        second_start
    );
    // Both targets were attempted exactly once; the first success must not
    // mask the second failure.
    assert_eq!(first_calls, 1);
    assert_eq!(second_calls, 1);

    let rendered = err.to_string();
    assert!(
        rendered.starts_with("document 2 "),
        "unexpected rendering: {}",
        rendered
    );
}

#[test]
fn unexpected_third_document_is_rejected_without_callback() {
    let input = "---\nname: Ada\n---\nx: 1\ny: 2\n---\nextra: true\n";

    let mut seen = Vec::new();

    let mut first_calls = 0usize;
    let mut third_calls = 0usize;

    let err = serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                first_calls += 1;
                seen.push(Person::deserialize(de)?.name);
                Ok(())
            },
            &mut |de| {
                third_calls += 1;
                Point::deserialize(de).map(drop)
            },
        ],
    )
    .unwrap_err();

    let doc = err.document().expect("error attributed to a document");
    assert_eq!(doc.document(), 3);
    let third_start = input.find("extra: true").unwrap();
    assert!(doc.start() <= third_start && doc.end() > third_start);
    assert_eq!(first_calls, 1);
    assert_eq!(third_calls, 1);
    assert_eq!(seen, vec!["Ada".to_owned()]);

    let rendered = err.to_string();
    assert!(
        rendered.contains("unexpected document 3"),
        "unexpected rendering: {}",
        rendered
    );
}

#[test]
fn fewer_documents_than_targets_is_diagnosed_and_unreached_not_called() {
    let input = "name: Ada\n";

    let mut first_calls = 0usize;
    let mut second_calls = 0usize;

    let err = serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                first_calls += 1;
                Person::deserialize(de).map(drop)
            },
            &mut |de| {
                second_calls += 1;
                Point::deserialize(de).map(drop)
            },
        ],
    )
    .unwrap_err();

    // A missing-document failure is not attributed to a document number.
    assert!(err.document().is_none());
    let msg = err.to_string();
    assert!(
        msg.contains("expected 2 YAML document(s), but found 1"),
        "unexpected message: {}",
        msg
    );
    assert_eq!(first_calls, 1);
    assert_eq!(second_calls, 0);
}

#[test]
fn empty_document_is_presented_as_null() {
    // An explicitly empty middle document, surrounded by content documents.
    let input = "x: 1\ny: 2\n---\n---\nname: Hopper\n";

    let mut middle: Option<Value> = Some(Value::Bool(true));
    let mut points = Vec::new();
    let mut people = Vec::new();

    serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                points.push(Point::deserialize(de)?);
                Ok(())
            },
            &mut |de| {
                middle = Some(Value::deserialize(de)?);
                Ok(())
            },
            &mut |de| {
                people.push(Person::deserialize(de)?);
                Ok(())
            },
        ],
    )
    .unwrap();

    assert_eq!(points, vec![Point { x: 1, y: 2 }]);
    assert_eq!(middle, Some(Value::Null));
    assert_eq!(
        people,
        vec![Person {
            name: "Hopper".to_owned()
        }]
    );

    // An empty first document deserializes into Option<T>::None and ().
    let input = "---\n...\n---\nx: 1\ny: 2\n";
    let mut unit: Option<()> = Some(());
    let mut p: Option<Point> = None;
    serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                unit = <Option<()>>::deserialize(de)?;
                Ok(())
            },
            &mut |de| {
                p = Some(Point::deserialize(de)?);
                Ok(())
            },
        ],
    )
    .unwrap();
    assert_eq!(unit, None);
    assert_eq!(p, Some(Point { x: 1, y: 2 }));
}

#[test]
fn anchors_do_not_leak_between_documents() {
    // Each document defines the same anchor name with a distinct value;
    // alias resolution must be scoped to a single document.
    let input = "a: &v 1\nb: *v\n---\na: &v 2\nb: *v\n";

    let mut first: Option<BTreeMap<String, u8>> = None;
    let mut second: Option<BTreeMap<String, u8>> = None;
    serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                first = Some(BTreeMap::<String, u8>::deserialize(de)?);
                Ok(())
            },
            &mut |de| {
                second = Some(BTreeMap::<String, u8>::deserialize(de)?);
                Ok(())
            },
        ],
    )
    .unwrap();

    assert_eq!(first.unwrap().get("b"), Some(&1u8));
    assert_eq!(second.unwrap().get("b"), Some(&2u8));
}

#[test]
fn parse_error_is_attributed_to_owning_document() {
    // Invalid YAML only in the second document.
    let input = "x: 7\ny: 8\n---\n[unterminated\n";
    let second_start = input.find("[unterminated").unwrap();

    let mut first_calls = 0usize;
    let mut second_calls = 0usize;

    let err = serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                first_calls += 1;
                Point::deserialize(de).map(drop)
            },
            &mut |de| {
                second_calls += 1;
                Value::deserialize(de).map(drop)
            },
        ],
    )
    .unwrap_err();

    let doc = err.document().expect("error attributed to a document");
    assert_eq!(doc.document(), 2);
    assert!(doc.start() <= second_start && doc.end() >= second_start);
    assert_eq!(first_calls, 1);
    // The second target receives the document; libyaml reports the parse
    // error through it. Either way the whole stream must fail.
    let _ = second_calls;
}

#[test]
fn zero_targets_with_a_document_is_rejected() {
    // Even an empty stream counts as one null document, so zero targets and
    // any input at all is an unexpected-document failure.
    let err = serde_yaml::from_str_documents("x: 1\n", &mut []).unwrap_err();
    let doc = err.document().expect("error attributed to a document");
    assert_eq!(doc.document(), 1);
    assert!(err.to_string().contains("unexpected document 1"));
}

#[test]
fn empty_input_matches_zero_targets_and_is_null_for_one() {
    // Empty input is a single implicit null document.
    serde_yaml::from_str_documents("", &mut []).unwrap_err();

    let mut value = Value::Bool(true);
    serde_yaml::from_str_documents(
        "",
        &mut [&mut |de| {
            value = Value::deserialize(de)?;
            Ok(())
        }],
    )
    .unwrap();
    assert_eq!(value, Value::Null);
}

#[test]
fn trailing_extra_empty_document_is_rejected() {
    // Two targets but three documents where the third is empty.
    let input = "x: 1\ny: 2\n---\nname: A\n---\n";

    let mut a = 0usize;
    let mut b = 0usize;
    let err = serde_yaml::from_str_documents(
        input,
        &mut [
            &mut |de| {
                a += 1;
                Point::deserialize(de).map(drop)
            },
            &mut |de| {
                b += 1;
                Person::deserialize(de).map(drop)
            },
        ],
    )
    .unwrap_err();

    let doc = err.document().expect("error attributed to a document");
    assert_eq!(doc.document(), 3);
    assert_eq!(a, 1);
    assert_eq!(b, 1);
}

#[test]
fn plain_from_str_is_unchanged() {
    // Single document still works.
    let point: Point = serde_yaml::from_str("x: 3\ny: 4").unwrap();
    assert_eq!(point, Point { x: 3, y: 4 });

    // Multiple documents in ordinary from_str remain rejected with the
    // original diagnostic. Use Value so that deserialization itself succeeds
    // and the multi-document guard is what fires (it runs after the first
    // document is read).
    let err: serde_yaml::Result<Value> = serde_yaml::from_str("x: 1\n---\nx: 2");
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("more than one document"),
        "unexpected message: {}",
        msg
    );

    // Empty input is null for Option types.
    let none: Option<u8> = serde_yaml::from_str("").unwrap();
    assert_eq!(none, None);
}
