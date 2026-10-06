#![allow(clippy::redundant_closure)]

use serde_derive::Deserialize;
use serde_yaml::{DocumentErrorKind, Documents, NullPolicy};
use std::cell::Cell;

#[derive(Debug, Deserialize, PartialEq)]
struct Meta {
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Payload {
    id: u32,
}

#[test]
fn two_documents_two_types() {
    let input = "---\nname: alpha\n---\nid: 42\n";
    let meta_calls = Cell::new(0);
    let payload_calls = Cell::new(0);

    let mut documents = Documents::from_str(input);
    documents
        .read(|meta: Meta| {
            meta_calls.set(meta_calls.get() + 1);
            assert_eq!(meta.name, "alpha");
        })
        .unwrap();
    documents
        .read(|payload: Payload| {
            payload_calls.set(payload_calls.get() + 1);
            assert_eq!(payload.id, 42);
        })
        .unwrap();
    documents.finish().unwrap();

    assert_eq!(meta_calls.get(), 1);
    assert_eq!(payload_calls.get(), 1);
}

#[test]
fn second_document_type_error_is_located_at_second_document() {
    let input = "---\nname: alpha\n---\nid: not-a-number\n";
    let meta_calls = Cell::new(0);
    let payload_calls = Cell::new(0);

    let mut documents = Documents::from_str(input);
    documents
        .read(|meta: Meta| {
            meta_calls.set(meta_calls.get() + 1);
            assert_eq!(meta.name, "alpha");
        })
        .unwrap();
    let err = documents
        .read(|_payload: Payload| {
            payload_calls.set(payload_calls.get() + 1);
        })
        .unwrap_err();

    // The failure is pinned to the second document.
    assert_eq!(err.document(), 2);
    assert!(matches!(err.kind(), DocumentErrorKind::Invalid(_)));

    // The reported input range falls inside the second document's bytes.
    let span = err.span().unwrap();
    let second_doc = input.find("id: not-a-number").unwrap();
    assert!(span.start <= second_doc, "span {:?} vs doc at {}", span, second_doc);
    assert!(span.end <= input.len());
    assert_eq!(&input[span.start..span.start + 2], "id");

    // The underlying error location points at the offending line (line 4).
    let location = err.location().unwrap();
    assert_eq!(location.line(), 4);

    // The failing target's callback never ran; the first ran exactly once.
    assert_eq!(meta_calls.get(), 1);
    assert_eq!(payload_calls.get(), 0);

    // Display keeps the document number.
    assert!(err.to_string().starts_with("YAML document 2: "), "{}", err);
}

#[test]
fn extra_third_document_is_rejected() {
    let input = "---\nname: alpha\n---\nid: 42\n---\nname: stowaway\n";
    let meta_calls = Cell::new(0);
    let payload_calls = Cell::new(0);

    let mut documents = Documents::from_str(input);
    documents
        .read(|_meta: Meta| meta_calls.set(meta_calls.get() + 1))
        .unwrap();
    documents
        .read(|_payload: Payload| payload_calls.set(payload_calls.get() + 1))
        .unwrap();

    // Two successful reads do not mean the stream is fully consumed.
    let err = documents.finish().unwrap_err();
    assert_eq!(err.document(), 3);
    assert!(matches!(err.kind(), DocumentErrorKind::ExtraDocument));
    let span = err.span().unwrap();
    assert_eq!(&input[span.start..span.start + 4], "name");

    assert_eq!(meta_calls.get(), 1);
    assert_eq!(payload_calls.get(), 1);
}

#[test]
fn missing_document_is_diagnosed_separately() {
    let input = "---\nname: alpha\n";
    let payload_calls = Cell::new(0);

    let mut documents = Documents::from_str(input);
    documents.read(|_meta: Meta| ()).unwrap();
    let err = documents
        .read(|_payload: Payload| payload_calls.set(payload_calls.get() + 1))
        .unwrap_err();

    assert_eq!(err.document(), 2);
    assert!(matches!(err.kind(), DocumentErrorKind::MissingDocument));
    assert_eq!(err.span(), None);
    assert_eq!(payload_calls.get(), 0);
}

#[test]
fn empty_document_follows_declared_null_policy() {
    // Three documents: a mapping, an empty document, another mapping.
    let input = "---\nname: alpha\n---\n---\nid: 7\n";

    // Default policy (Null): the empty document deserializes as YAML null.
    let mut documents = Documents::from_str(input);
    let mut seen = Vec::new();
    documents
        .read(|meta: Meta| seen.push(format!("meta:{}", meta.name)))
        .unwrap();
    documents
        .read(|gap: Option<Payload>| seen.push(format!("gap:{:?}", gap)))
        .unwrap();
    documents
        .read(|payload: Payload| seen.push(format!("payload:{}", payload.id)))
        .unwrap();
    documents.finish().unwrap();
    assert_eq!(seen, vec!["meta:alpha", "gap:None", "payload:7"]);

    // Declared Reject policy: the empty document is an error naming
    // document 2, and the target's Deserialize is never invoked.
    let mut documents = Documents::from_str(input);
    documents.null_policy(NullPolicy::Reject);
    documents.read(|_meta: Meta| ()).unwrap();
    let err = documents.read(|_gap: Option<Payload>| ()).unwrap_err();
    assert_eq!(err.document(), 2);
    assert!(matches!(err.kind(), DocumentErrorKind::Empty));
    assert!(err.span().is_some());
    assert_eq!(
        err.to_string(),
        "YAML document 2: empty document rejected by the declared null policy"
    );
}

#[test]
fn tag_state_does_not_leak_between_documents() {
    // `!!str` forces the first document's scalar to be a string. If tag
    // state leaked, the second document's plain `123` would not parse as
    // an integer.
    let input = "---\n!!str 123\n---\n123\n";
    let mut documents = Documents::from_str(input);
    documents
        .read(|s: String| assert_eq!(s, "123"))
        .unwrap();
    documents.read(|n: u32| assert_eq!(n, 123)).unwrap();
    documents.finish().unwrap();
}

#[test]
fn anchors_do_not_leak_between_documents() {
    // The alias `*a` in the second document refers to an anchor defined in
    // the first document; each document is parsed independently, so this
    // must fail on document 2 rather than resolving.
    let input = "---\n- &a 1\n---\n*a\n";
    let mut documents = Documents::from_str(input);
    documents
        .read(|v: Vec<u32>| assert_eq!(v, vec![1]))
        .unwrap();
    let err = documents
        .read(|_v: serde_yaml::Value| ())
        .unwrap_err();
    assert_eq!(err.document(), 2);
    assert!(matches!(err.kind(), DocumentErrorKind::Invalid(_)));
    assert!(err.to_string().contains("unknown anchor"), "{}", err);
}

#[test]
fn unselected_targets_do_not_run_deserialize() {
    // After the second document fails, later targets are never selected:
    // no user Deserialize code runs for them and no callbacks fire.
    let input = "---\nname: alpha\n---\nid: oops\n---\nname: beta\n";
    let calls = Cell::new(0);

    let mut documents = Documents::from_str(input);
    documents.read(|_meta: Meta| calls.set(calls.get() + 1)).unwrap();
    let err = documents.read(|_payload: Payload| calls.set(calls.get() + 1));
    assert_eq!(err.unwrap_err().document(), 2);
    assert_eq!(calls.get(), 1);
}

#[test]
fn ordinary_from_str_is_unchanged() {
    // Single document still deserializes directly.
    let meta: Meta = serde_yaml::from_str("name: alpha\n").unwrap();
    assert_eq!(meta.name, "alpha");

    // Multi-document input is still rejected as a whole by from_str.
    let err = serde_yaml::from_str::<Meta>("---\nname: alpha\n---\nid: 1\n").unwrap_err();
    assert!(
        err.to_string().contains("more than one document"),
        "{}",
        err
    );
}
