#![allow(clippy::unwrap_used)]

use indoc::indoc;
use serde::Deserialize;
use serde_yaml::{Deserializer, ScalarStyle};

/// Plain number deserializes to an integer successfully; the successful
/// value is a plain `i64` and carries no extra structure.
#[test]
fn test_plain_number_to_int_ok() {
    let value: i64 = serde_yaml::from_str("123").unwrap();
    assert_eq!(value, 123);

    let value: u64 = serde_yaml::from_str("123").unwrap();
    assert_eq!(value, 123);
}

/// Plain non-numeric scalar fails to deserialize as an integer; the error
/// is diagnosed as a plain scalar and keeps its line/column and cause.
#[test]
fn test_plain_scalar_error_style() {
    let yaml = indoc! {"
        ---
        abc
    "};
    let err = serde_yaml::from_str::<i64>(yaml).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Plain));

    let location = err.location().unwrap();
    assert_eq!(location.line(), 2);
    assert_eq!(location.column(), 1);

    let cause = "invalid type: string \"abc\", expected i64 at line 2 column 1";
    assert_eq!(err.to_string(), cause);
}

/// Quoted numbers (single or double quotes) fail to deserialize as an
/// integer; the error is diagnosed as a quoted scalar. The same input
/// deserializes to a string successfully.
#[test]
fn test_quoted_number_to_int_error_style() {
    for yaml in ["\"123\"", "'123'"] {
        let err = serde_yaml::from_str::<i64>(yaml).unwrap_err();
        assert_eq!(err.scalar_style(), Some(ScalarStyle::Quoted));

        let location = err.location().unwrap();
        assert_eq!(location.line(), 1);
        assert_eq!(location.column(), 1);

        assert!(err.to_string().contains("invalid type: string \"123\""));

        // Target type conversion is unchanged: the quoted number is still
        // a perfectly good string.
        let string: String = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(string, "123");
    }
}

/// Block scalars (literal `|` and folded `>`) fail to deserialize as an
/// integer; the error is diagnosed as a block scalar.
#[test]
fn test_block_scalar_to_int_error_style() {
    let yaml = indoc! {"
        |
          123
    "};
    let err = serde_yaml::from_str::<i64>(yaml).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Block));
    let location = err.location().unwrap();
    assert_eq!(location.line(), 1);
    assert_eq!(location.column(), 1);
    assert!(err.to_string().contains("expected i64"));

    let yaml = indoc! {"
        >
          123
    "};
    let err = serde_yaml::from_str::<i64>(yaml).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Block));

    // The block content itself is a valid string.
    let string: String = serde_yaml::from_str("|\n  123\n").unwrap();
    assert_eq!(string, "123\n");
}

/// Non-scalar errors carry no scalar style hint.
#[test]
fn test_non_scalar_error_has_no_style() {
    let err = serde_yaml::from_str::<i64>("[1, 2]").unwrap_err();
    assert_eq!(err.scalar_style(), None);

    let err = serde_yaml::from_str::<i64>("k: 1").unwrap_err();
    assert_eq!(err.scalar_style(), None);
}

/// In a multi-document stream, each document's error reports its own
/// location and scalar style; the second document never inherits the
/// style of a scalar from the first document.
#[test]
fn test_multi_document_style_and_position_independence() {
    let yaml = indoc! {"
        ---
        |
          123
        ...
        ---
        abc
    "};

    let mut de = Deserializer::from_str(yaml);

    let first = de.next().unwrap();
    let err = i64::deserialize(first).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Block));
    let location = err.location().unwrap();
    assert_eq!(location.line(), 2);
    assert_eq!(location.column(), 1);

    let second = de.next().unwrap();
    let err = i64::deserialize(second).unwrap_err();
    // Style of the second document's own scalar, not the block style of
    // the first document.
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Plain));
    // Position is independent of the first document.
    let location = err.location().unwrap();
    assert_eq!(location.line(), 6);
    assert_eq!(location.column(), 1);
    assert_eq!(
        err.to_string(),
        "invalid type: string \"abc\", expected i64 at line 6 column 1",
    );

    assert!(de.next().is_none());
}

/// Same behavior through a temporary-file fixture and the reader-based
/// interface.
#[test]
fn test_temp_file_fixture() {
    let yaml = "---\n\"123\"\n...\n---\nabc\n";

    let mut path = std::env::temp_dir();
    path.push(format!("serde_yaml_scalar_style_{}.yaml", std::process::id()));
    std::fs::write(&path, yaml).unwrap();

    let file = std::fs::File::open(&path).unwrap();
    let mut de = Deserializer::from_reader(file);

    let first = de.next().unwrap();
    let err = i64::deserialize(first).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Quoted));
    let location = err.location().unwrap();
    assert_eq!(location.line(), 2);
    assert_eq!(location.column(), 1);

    let second = de.next().unwrap();
    let err = i64::deserialize(second).unwrap_err();
    assert_eq!(err.scalar_style(), Some(ScalarStyle::Plain));
    let location = err.location().unwrap();
    assert_eq!(location.line(), 5);
    assert_eq!(location.column(), 1);

    assert!(de.next().is_none());
    drop(de);

    std::fs::remove_file(&path).unwrap();
}
