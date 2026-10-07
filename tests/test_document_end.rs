#![allow(clippy::uninlined_format_args)]

use indoc::indoc;
use serde::Deserialize;
use serde_yaml::{Deserializer, Value};

fn strict_documents(yaml: &str) -> Deserializer {
    Deserializer::from_str(yaml).require_explicit_document_end()
}

#[test]
fn test_explicit_end_single_document() {
    let yaml = "k: 107\n...\n";
    let mut documents = strict_documents(yaml);
    let value = Value::deserialize(documents.next().unwrap()).unwrap();
    assert_eq!(value["k"], 107);
    assert!(documents.next().is_none());
}

#[test]
fn test_implicit_end_at_eof() {
    let yaml = "k: 107\n";
    let mut documents = strict_documents(yaml);
    let err = Value::deserialize(documents.next().unwrap()).unwrap_err();
    assert!(
        err.to_string().contains("document 1"),
        "unexpected error: {}",
        err,
    );
    assert!(documents.next().is_none());
}

#[test]
fn test_two_documents_first_missing_end() {
    let yaml = indoc! {"
        ---
        a: 1
        ---
        b: 2
        ...
    "};
    let mut documents = strict_documents(yaml);

    // First document is terminated implicitly by the second `---`.
    let err = Value::deserialize(documents.next().unwrap()).unwrap_err();
    assert!(
        err.to_string().contains("document 1"),
        "unexpected error: {}",
        err,
    );

    // The error must not swallow the start of the next document.
    let second = Value::deserialize(documents.next().unwrap()).unwrap();
    assert_eq!(second["b"], 2);
    assert!(documents.next().is_none());
}

#[test]
fn test_two_documents_both_missing_end() {
    let yaml = indoc! {"
        ---
        a: 1
        ---
        b: 2
    "};
    let mut documents = strict_documents(yaml);

    let err = Value::deserialize(documents.next().unwrap()).unwrap_err();
    assert!(
        err.to_string().contains("document 1"),
        "unexpected error: {}",
        err,
    );

    let err = Value::deserialize(documents.next().unwrap()).unwrap_err();
    assert!(
        err.to_string().contains("document 2"),
        "unexpected error: {}",
        err,
    );
    assert!(documents.next().is_none());
}

#[test]
fn test_block_scalar_containing_dots() {
    // A `...` line inside a block scalar is scalar content, not a document
    // end marker; the document still needs its own explicit `...`.
    let yaml = indoc! {"
        k: |
          ...
          text
        ...
    "};
    let mut documents = strict_documents(yaml);
    let value = Value::deserialize(documents.next().unwrap()).unwrap();
    assert_eq!(value["k"], "...\ntext\n");
    assert!(documents.next().is_none());
}

#[test]
fn test_block_scalar_dots_do_not_count_as_end() {
    // The `...` inside the block scalar must not satisfy strict mode; this
    // document ends implicitly at EOF and must be reported.
    let yaml = indoc! {"
        k: |
          ...
    "};
    let mut documents = strict_documents(yaml);
    let err = Value::deserialize(documents.next().unwrap()).unwrap_err();
    assert!(
        err.to_string().contains("document 1"),
        "unexpected error: {}",
        err,
    );
}

#[test]
fn test_default_parsing_unchanged() {
    // Without strict mode, implicit document ends remain accepted.
    let yaml = "k: 107\n";
    let value: Value = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(value["k"], 107);

    let yaml = "---\na: 1\n---\nb: 2\n";
    let mut documents = Deserializer::from_str(yaml);
    let first = Value::deserialize(documents.next().unwrap()).unwrap();
    assert_eq!(first["a"], 1);
    let second = Value::deserialize(documents.next().unwrap()).unwrap();
    assert_eq!(second["b"], 2);
    assert!(documents.next().is_none());
}
