//! Explicit type-source reporting for YAML deserialization.
//!
//! When this reporting is enabled, the deserializer records one [`TypeSource`]
//! entry every time it actually encounters a scalar, sequence, mapping or
//! explicit `!tag` node, together with the type that was expected at that
//! point and the position of the node in the input. Nodes reached through a
//! YAML alias (`*alias`) additionally carry both the position where the anchor
//! (`&anchor`) is defined and the position where the alias is referenced.
//!
//! Reporting is done during the single normal deserialization pass. The
//! user's `Deserialize` implementation is executed exactly once; there is no
//! hidden second pass that re-deserializes the input to guess positions.
//! Positions that are unknown are reported as `None` and never fabricated as
//! line 0. Errors produced by the user's own `Deserialize` implementation
//! keep their original message, and the reporting API is compatible with both
//! the `Ok` and the `Err` outcome of deserialization.
//!
//! # Verifying locally
//!
//! ```console
//! cargo test --test test_report
//! cargo test --doc report
//! ```
//!
//! # Error examples
//!
//! A field that expects an integer but meets a sequence is attributed to the
//! sequence node itself:
//!
//! ```
//! # use serde_derive::Deserialize;
//! use serde::Deserialize;
//! use serde_yaml::report::TypeReporter;
//! use serde_yaml::Deserializer;
//!
//! #[derive(Deserialize, Debug)]
//! struct Config {
//!     port: u16,
//! }
//!
//! let yaml = "port:\n  - 1\n  - 2\n";
//! let reporter = TypeReporter::new();
//! let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
//! let err = Config::deserialize(de).unwrap_err();
//! assert_eq!(
//!     err.to_string(),
//!     "port: invalid type: sequence, expected u16 at line 2 column 3",
//! );
//! let report = reporter.report();
//! let entry = report.entries().last().unwrap();
//! assert_eq!(entry.expected(), "u16");
//! assert_eq!(entry.location().unwrap().line(), 2);
//! ```
//!
//! An error behind an alias points at both the alias reference and the
//! anchor definition:
//!
//! ```
//! # use serde_derive::Deserialize;
//! # use serde::Deserialize;
//! # use serde_yaml::report::TypeReporter;
//! # use serde_yaml::Deserializer;
//! #[derive(Deserialize, Debug)]
//! struct Config {
//!     a: Vec<u16>,
//!     b: u16,
//! }
//!
//! let yaml = "a: &x [1, 2]\nb: *x\n";
//! let reporter = TypeReporter::new();
//! let de = Deserializer::from_str(yaml).with_type_reporter(&reporter);
//! let err = Config::deserialize(de).unwrap_err();
//! assert_eq!(
//!     err.to_string(),
//!     "b: invalid type: sequence, expected u16 at line 1 column 4 \
//!      (alias defined at line 1 column 4, referenced at line 2 column 4)",
//! );
//! ```
//!
//! Positions from multiple documents are not mixed up:
//!
//! ```
//! # use serde_derive::Deserialize;
//! # use serde::Deserialize;
//! # use serde_yaml::report::TypeReporter;
//! # use serde_yaml::Deserializer;
//! # #[derive(Deserialize, Debug)]
//! # struct Config {
//! #     port: u16,
//! # }
//! let yaml = "---\nport: 1\n---\nport:\n  - 1\n";
//! let reporter = TypeReporter::new();
//! let mut documents = Deserializer::from_str(yaml).with_type_reporter(&reporter);
//! assert!(Config::deserialize(documents.next().unwrap()).is_ok());
//! let err = Config::deserialize(documents.next().unwrap()).unwrap_err();
//! assert_eq!(
//!     err.to_string(),
//!     "port: invalid type: sequence, expected u16 at line 5 column 3",
//! );
//! ```

use crate::error::Location;
use std::cell::RefCell;
use std::rc::Rc;

/// The kind of YAML node that was encountered while deserializing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A scalar node, for example `107` or `"hello"`.
    Scalar,
    /// A sequence node, for example `[1, 2]`.
    Sequence,
    /// A mapping node, for example `{k: v}`.
    Mapping,
    /// An explicit YAML `!tag` attached to a node.
    Tag,
}

/// The two positions involved in a YAML alias reference: where the anchored
/// value is defined and where the alias is referenced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AliasPositions {
    definition: Location,
    reference: Location,
}

impl AliasPositions {
    pub(crate) fn new(definition: Location, reference: Location) -> Self {
        AliasPositions {
            definition,
            reference,
        }
    }

    /// The position at which the anchored value (`&anchor`) is defined.
    pub fn definition(&self) -> Location {
        self.definition
    }

    /// The position at which the alias (`*alias`) is referenced.
    pub fn reference(&self) -> Location {
        self.reference
    }
}

/// One recorded encounter of a YAML node with the type that was expected at
/// that point during deserialization.
#[derive(Clone, Debug)]
pub struct TypeSource {
    expected: String,
    node: NodeKind,
    location: Option<Location>,
    alias: Option<AliasPositions>,
}

impl TypeSource {
    pub(crate) fn new(
        expected: String,
        node: NodeKind,
        location: Option<Location>,
        alias: Option<AliasPositions>,
    ) -> Self {
        TypeSource {
            expected,
            node,
            location,
            alias,
        }
    }

    /// The type that the deserializer was expecting at this node, rendered
    /// the same way as in the `expected ...` part of error messages.
    pub fn expected(&self) -> &str {
        &self.expected
    }

    /// The kind of YAML node that was encountered.
    pub fn node(&self) -> NodeKind {
        self.node
    }

    /// The position of the encountered node in the input.
    ///
    /// This is `None` when the position is unknown; serde_yaml never
    /// fabricates a line 0 for an unknown position.
    pub fn location(&self) -> Option<Location> {
        self.location
    }

    /// If this node was reached through a YAML alias, the positions of both
    /// the alias reference and the anchor definition.
    pub fn alias(&self) -> Option<AliasPositions> {
        self.alias
    }
}

/// The recorded type-source entries of one or more deserialization runs.
#[derive(Clone, Debug, Default)]
pub struct TypeReport {
    entries: Vec<TypeSource>,
}

impl TypeReport {
    /// All recorded entries, in the order the nodes were encountered.
    pub fn entries(&self) -> &[TypeSource] {
        &self.entries
    }

    /// An iterator over the recorded entries.
    pub fn iter(&self) -> impl Iterator<Item = &TypeSource> {
        self.entries.iter()
    }

    /// The number of recorded entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no entries were recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A handle that collects [`TypeSource`] entries during deserialization.
///
/// Attach a reporter to a [`Deserializer`][crate::Deserializer] with
/// [`Deserializer::with_type_reporter`][crate::Deserializer::with_type_reporter]
/// and deserialize as usual. The reporter works the same whether
/// deserialization succeeds or fails, and can be shared across the documents
/// of a multi-document stream; entries recorded for one document always carry
/// that document's own positions.
#[derive(Clone, Default)]
pub struct TypeReporter {
    inner: Rc<RefCell<Vec<TypeSource>>>,
}

impl TypeReporter {
    /// Creates an empty reporter.
    pub fn new() -> Self {
        TypeReporter {
            inner: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Returns a snapshot of everything recorded so far.
    pub fn report(&self) -> TypeReport {
        TypeReport {
            entries: self.inner.borrow().clone(),
        }
    }

    /// Drops all entries recorded so far.
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    pub(crate) fn push(&self, entry: TypeSource) {
        self.inner.borrow_mut().push(entry);
    }
}
