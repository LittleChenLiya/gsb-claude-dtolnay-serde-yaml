//! Explicit typed sequence over a multi-document YAML stream.
//!
//! [`Documents`] walks a YAML stream document by document. Every document is
//! deserialized independently into its own explicitly registered target type:
//! no parser, tag, anchor or enum state leaks from one document into the
//! next, and the success of an earlier document is never treated as success
//! of the whole stream.
//!
//! The number of documents in the stream is checked against the number of
//! registered targets in both directions: a stream that ends early is
//! reported as [`DocumentErrorKind::MissingDocument`], while trailing
//! documents beyond the declared type list are rejected as
//! [`DocumentErrorKind::ExtraDocument`] by [`Documents::finish`]. Targets
//! whose document is never reached (because an earlier document failed) do
//! not run any user `Deserialize` code.
//!
//! # Three-document example
//!
//! ```
//! use serde_derive::Deserialize;
//! use serde_yaml::{Documents, NullPolicy};
//!
//! #[derive(Debug, Deserialize, PartialEq)]
//! struct Config {
//!     name: String,
//! }
//!
//! #[derive(Debug, Deserialize, PartialEq)]
//! struct Record {
//!     id: u32,
//! }
//!
//! fn main() -> Result<(), serde_yaml::DocumentError> {
//!     // Three documents: a mapping, an empty document, another mapping.
//!     let input = "---\nname: demo\n---\n---\nid: 7\n";
//!
//!     let mut documents = Documents::from_str(input);
//!     // Declare how empty documents are treated. NullPolicy::Null (the
//!     // default) deserializes an empty document as YAML null;
//!     // NullPolicy::Reject would report it as an error naming document 2.
//!     documents.null_policy(NullPolicy::Null);
//!
//!     documents.read(|config: Config| assert_eq!(config.name, "demo"))?;
//!     documents.read(|gap: Option<Record>| assert_eq!(gap, None))?;
//!     documents.read(|record: Record| assert_eq!(record.id, 7))?;
//!     documents.finish()?; // rejects any extra document beyond the list
//!     Ok(())
//! }
//! ```

use crate::de::{Deserializer, Event, Progress};
use crate::error::{self, Error, ErrorImpl, Location};
use crate::libyaml::parser::ScalarStyle;
use crate::loader::{Document, Loader};
use serde::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display};
use std::io;
use std::ops::Range;
use std::sync::Arc;

/// Declared strategy for empty documents in a [`Documents`] stream.
///
/// An empty document is one with no content at all between its document
/// markers, for example the middle document of `"---\na: 1\n---\n---\nb: 2\n"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NullPolicy {
    /// Deserialize an empty document as YAML null. This is the default and
    /// matches the behavior of [`crate::Deserializer`]: an empty document
    /// produces `None` for `Option<T>`, [`crate::Value::Null`] for
    /// `Value`, and a type error for targets that do not accept null.
    Null,
    /// Reject an empty document with [`DocumentErrorKind::Empty`], reporting
    /// its document number and input range instead of running the target
    /// type's `Deserialize` implementation on it.
    Reject,
}

impl Default for NullPolicy {
    fn default() -> Self {
        NullPolicy::Null
    }
}

/// A reader that deserializes each document of a multi-document YAML stream
/// into its own explicitly registered target type.
///
/// See the [module-level documentation](self) for an overview and example.
pub struct Documents<'de> {
    loader: std::result::Result<Loader<'de>, Arc<ErrorImpl>>,
    /// 1-based number of the document the next `read` will consume.
    next: usize,
    null_policy: NullPolicy,
}

impl<'de> Documents<'de> {
    /// Creates a typed document reader from a `&str`.
    pub fn from_str(s: &'de str) -> Self {
        Self::from_progress(Progress::Str(s))
    }

    /// Creates a typed document reader from a `&[u8]`.
    pub fn from_slice(v: &'de [u8]) -> Self {
        Self::from_progress(Progress::Slice(v))
    }

    /// Creates a typed document reader from an `io::Read`.
    ///
    /// Reader-based document readers do not support deserializing borrowed
    /// types like `&str`, since the `std::io::Read` trait has no non-copying
    /// methods -- everything it does involves copying bytes out of the data
    /// source.
    pub fn from_reader<R>(rdr: R) -> Self
    where
        R: io::Read + 'de,
    {
        Self::from_progress(Progress::Read(Box::new(rdr)))
    }

    fn from_progress(progress: Progress<'de>) -> Self {
        Documents {
            loader: Loader::new(progress).map_err(Error::shared),
            next: 1,
            null_policy: NullPolicy::default(),
        }
    }

    /// Declares the strategy applied to empty documents. See [`NullPolicy`].
    ///
    /// The default is [`NullPolicy::Null`].
    pub fn null_policy(&mut self, policy: NullPolicy) -> &mut Self {
        self.null_policy = policy;
        self
    }

    /// Deserializes the next document of the stream as `T` and passes the
    /// resulting value to `f`.
    ///
    /// Each call creates a fresh deserializer for the next document only, so
    /// tags, anchors and enum state from earlier documents cannot leak into
    /// this one. The callback is invoked exactly once if, and only if, the
    /// document deserializes successfully; on failure no user `Deserialize`
    /// code runs for this or any later (unselected) target unless `read` is
    /// called again.
    ///
    /// A successful call says nothing about the rest of the stream; call
    /// [`Documents::finish`] after the last registered target to reject
    /// trailing extra documents.
    pub fn read<T, R>(&mut self, f: impl FnOnce(T) -> R) -> Result<R, DocumentError>
    where
        T: Deserialize<'de>,
    {
        let document_number = self.next;
        let loader = match &mut self.loader {
            Ok(loader) => loader,
            Err(shared) => {
                return Err(DocumentError {
                    document: document_number,
                    span: None,
                    kind: DocumentErrorKind::Invalid(error::shared(Arc::clone(shared))),
                });
            }
        };
        let document = match loader.next_document() {
            Some(document) => document,
            None => {
                return Err(DocumentError {
                    document: document_number,
                    span: None,
                    kind: DocumentErrorKind::MissingDocument,
                });
            }
        };
        self.next += 1;
        let span = document_span(&document);
        if self.null_policy == NullPolicy::Reject && is_empty_document(&document) {
            return Err(DocumentError {
                document: document_number,
                span,
                kind: DocumentErrorKind::Empty,
            });
        }
        match T::deserialize(Deserializer::from_document(document)) {
            Ok(value) => Ok(f(value)),
            Err(err) => Err(DocumentError {
                document: document_number,
                span,
                kind: DocumentErrorKind::Invalid(err),
            }),
        }
    }

    /// Verifies that the stream contains no documents beyond the ones
    /// consumed by [`Documents::read`].
    ///
    /// Returns [`DocumentErrorKind::ExtraDocument`] naming the first
    /// unexpected document if the stream contains more documents than the
    /// declared type list. The extra document is never deserialized.
    pub fn finish(&mut self) -> Result<(), DocumentError> {
        let loader = match &mut self.loader {
            Ok(loader) => loader,
            Err(shared) => {
                return Err(DocumentError {
                    document: self.next,
                    span: None,
                    kind: DocumentErrorKind::Invalid(error::shared(Arc::clone(shared))),
                });
            }
        };
        match loader.next_document() {
            None => Ok(()),
            Some(document) => Err(DocumentError {
                document: self.next,
                span: document_span(&document),
                kind: DocumentErrorKind::ExtraDocument,
            }),
        }
    }
}

/// Byte range of a document's events within the original input.
fn document_span(document: &Document) -> Option<Range<usize>> {
    let first = document.events.first()?;
    let last = document.events.last()?;
    let start = first.1.index() as usize;
    let end = last.1.index() as usize;
    Some(start..end.max(start))
}

/// Whether the document has no content (an empty plain scalar, which is what
/// libyaml reports for a document with no content, or the void document
/// produced for empty input).
fn is_empty_document(document: &Document) -> bool {
    match document.events.as_slice() {
        [] => true,
        [(Event::Void, _)] => true,
        [(Event::Scalar(scalar), _)] => {
            scalar.tag.is_none()
                && scalar.style == ScalarStyle::Plain
                && scalar.value.is_empty()
        }
        _ => false,
    }
}

/// An error encountered while reading one document of a typed document
/// sequence. Always carries the 1-based document number and, when the
/// document was found in the input, its byte range within the input.
#[derive(Debug)]
pub struct DocumentError {
    document: usize,
    span: Option<Range<usize>>,
    kind: DocumentErrorKind,
}

/// What went wrong with a document in a typed document sequence.
#[derive(Debug)]
pub enum DocumentErrorKind {
    /// The document failed to parse, or failed to deserialize into its
    /// registered target type. Contains the underlying [`Error`], which
    /// carries the location within the input.
    Invalid(Error),
    /// The document is empty and the declared [`NullPolicy`] is
    /// [`NullPolicy::Reject`].
    Empty,
    /// The stream ended before this document: fewer YAML documents than the
    /// declared type list.
    MissingDocument,
    /// The stream contains more YAML documents than the declared type list.
    /// Reported by [`Documents::finish`]; the extra document is not
    /// deserialized.
    ExtraDocument,
}

impl DocumentError {
    /// The 1-based number of the document that failed, within the YAML
    /// stream.
    pub fn document(&self) -> usize {
        self.document
    }

    /// The byte range covered by the failing document within the original
    /// input, if the document was present in the stream.
    pub fn span(&self) -> Option<Range<usize>> {
        self.span.clone()
    }

    /// Which kind of failure occurred.
    pub fn kind(&self) -> &DocumentErrorKind {
        &self.kind
    }

    /// The location within the input, if the underlying error carries one.
    pub fn location(&self) -> Option<Location> {
        match &self.kind {
            DocumentErrorKind::Invalid(err) => err.location(),
            _ => None,
        }
    }
}

impl Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "YAML document {}: ", self.document)?;
        match &self.kind {
            DocumentErrorKind::Invalid(err) => Display::fmt(err, f),
            DocumentErrorKind::Empty => {
                f.write_str("empty document rejected by the declared null policy")
            }
            DocumentErrorKind::MissingDocument => {
                f.write_str("stream ended: fewer YAML documents than the declared type list")
            }
            DocumentErrorKind::ExtraDocument => {
                f.write_str("unexpected extra YAML document beyond the declared type list")
            }
        }
    }
}

impl StdError for DocumentError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.kind {
            DocumentErrorKind::Invalid(err) => Some(err),
            _ => None,
        }
    }
}
