use crate::libyaml::{emitter, error as libyaml};
use crate::path::Path;
use serde::{de, ser};
use std::error::Error as StdError;
use std::fmt::{self, Debug, Display};
use std::io;
use std::result;
use std::string;
use std::sync::Arc;

/// An error that happened serializing or deserializing YAML data.
pub struct Error(Box<ErrorImpl>);

/// Alias for a `Result` with the error type `serde_yaml::Error`.
pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
pub(crate) enum ErrorImpl {
    Message(String, Option<Pos>),
    TypeMismatch(Box<TypeMismatchImpl>),

    Libyaml(libyaml::Error),
    Io(io::Error),
    FromUtf8(string::FromUtf8Error),

    EndOfStream,
    MoreThanOneDocument,
    RecursionLimitExceeded(libyaml::Mark),
    RepetitionLimitExceeded,
    BytesUnsupported,
    UnknownAnchor(libyaml::Mark),
    SerializeNestedEnum,
    ScalarInMerge,
    TaggedInMerge,
    ScalarInMergeElement,
    SequenceInMergeElement,
    EmptyTag,
    FailedToParseNumber,

    Shared(Arc<ErrorImpl>),
}

#[derive(Debug)]
pub(crate) struct Pos {
    mark: libyaml::Mark,
    path: String,
    /// Position of the alias reference through which the node at `mark` was
    /// reached, if any.
    alias: Option<libyaml::Mark>,
}

/// A type mismatch between the YAML node encountered during deserialization
/// and the type expected by the `Deserialize` implementation.
#[derive(Debug)]
pub(crate) struct TypeMismatchImpl {
    /// The full error message, rendered the same way serde renders
    /// `invalid_type` errors.
    pub(crate) msg: String,
    /// Description of the expected type, like `i64`.
    pub(crate) expected: String,
    /// The kind of YAML node that was found instead.
    pub(crate) found: NodeKind,
    pub(crate) pos: Option<Pos>,
}

/// The kind of YAML node that was encountered at a position where the
/// `Deserialize` implementation expected a different type.
///
/// Returned by [`TypeSource::found`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A plain scalar node.
    Scalar,
    /// A sequence node.
    Sequence,
    /// A mapping node.
    Mapping,
    /// A node carrying an explicit YAML tag.
    Tagged,
}

/// An explicit report of where a deserialization type mismatch came from:
/// the expected type, the kind and position of the YAML node that was found
/// instead, and—when that node was reached through an alias—both the alias
/// reference position and the anchored definition position.
///
/// Returned by [`Error::type_source`].
#[derive(Debug)]
pub struct TypeSource {
    expected: String,
    found: NodeKind,
    mark: Option<libyaml::Mark>,
    alias_reference: Option<libyaml::Mark>,
}

impl TypeSource {
    /// A description of the type expected by the `Deserialize`
    /// implementation, like `i64` or `a boolean`.
    pub fn expected(&self) -> &str {
        &self.expected
    }

    /// The kind of YAML node that was found instead of the expected type.
    pub fn found(&self) -> NodeKind {
        self.found
    }

    /// The position of the YAML node that did not match the expected type.
    ///
    /// When the node was reached through an alias, this is the position of
    /// the anchored definition. Not all errors have a position.
    pub fn location(&self) -> Option<Location> {
        self.mark.map(Location::from_mark)
    }

    /// If the mismatched node was reached through an alias, the positions of
    /// both the alias reference and the anchored definition.
    pub fn alias(&self) -> Option<AliasLocations> {
        Some(AliasLocations {
            reference: Location::from_mark(self.alias_reference?),
            definition: Location::from_mark(self.mark?),
        })
    }
}

/// The two positions involved in following a YAML alias: where the alias
/// was referenced and where the anchored node was defined.
///
/// Returned by [`TypeSource::alias`].
#[derive(Debug, Clone, Copy)]
pub struct AliasLocations {
    reference: Location,
    definition: Location,
}

impl AliasLocations {
    /// The position of the alias reference (`*name`) in the input.
    pub fn reference(&self) -> Location {
        self.reference
    }

    /// The position of the anchored node definition (`&name`) in the input.
    pub fn definition(&self) -> Location {
        self.definition
    }
}

/// The input location that an error occured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    index: usize,
    line: usize,
    column: usize,
}

impl Location {
    /// The byte index of the error
    pub fn index(&self) -> usize {
        self.index
    }

    /// The line of the error
    pub fn line(&self) -> usize {
        self.line
    }

    /// The column of the error
    pub fn column(&self) -> usize {
        self.column
    }

    // This is to keep decoupled with the yaml crate
    #[doc(hidden)]
    fn from_mark(mark: libyaml::Mark) -> Self {
        Location {
            index: mark.index() as usize,
            // `line` and `column` returned from libyaml are 0-indexed but all error messages add +1 to this value
            line: mark.line() as usize + 1,
            column: mark.column() as usize + 1,
        }
    }
}

impl Error {
    /// Returns the Location from the error if one exists.
    ///
    /// Not all types of errors have a location so this can return `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use serde_yaml::{Value, Error};
    /// #
    /// // The `@` character as the first character makes this invalid yaml
    /// let invalid_yaml: Result<Value, Error> = serde_yaml::from_str("@invalid_yaml");
    ///
    /// let location = invalid_yaml.unwrap_err().location().unwrap();
    ///
    /// assert_eq!(location.line(), 1);
    /// assert_eq!(location.column(), 1);
    /// ```
    pub fn location(&self) -> Option<Location> {
        self.0.location()
    }

    /// If this error is a type mismatch between the YAML document and the
    /// type expected by the `Deserialize` implementation, returns an
    /// explicit report of the expected type and of the kind and position of
    /// the YAML node that was found instead.
    ///
    /// When the mismatched node was reached through a YAML alias, the report
    /// carries both the position of the alias reference and the position of
    /// the anchored definition; see [`TypeSource::alias`].
    ///
    /// The report is produced during the single deserialization pass that
    /// encountered the mismatch; the input is never deserialized a second
    /// time to reconstruct it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use serde_derive::Deserialize;
    /// # use serde_yaml::NodeKind;
    /// #[derive(Debug, Deserialize)]
    /// struct S {
    ///     x: i64,
    /// }
    ///
    /// let err = serde_yaml::from_str::<S>("x: [1, 2]\n").unwrap_err();
    /// let source = err.type_source().unwrap();
    /// assert_eq!(source.expected(), "i64");
    /// assert_eq!(source.found(), NodeKind::Sequence);
    /// assert_eq!(source.location().unwrap().line(), 1);
    /// ```
    pub fn type_source(&self) -> Option<TypeSource> {
        self.0.type_source()
    }
}

pub(crate) fn new(inner: ErrorImpl) -> Error {
    Error(Box::new(inner))
}

pub(crate) fn shared(shared: Arc<ErrorImpl>) -> Error {
    Error(Box::new(ErrorImpl::Shared(shared)))
}

pub(crate) fn fix_mark(mut error: Error, mark: libyaml::Mark, path: Path) -> Error {
    let pos = Pos {
        mark,
        path: path.to_string(),
        alias: None,
    };
    match error.0.as_mut() {
        ErrorImpl::Message(_, none @ None) => *none = Some(pos),
        ErrorImpl::TypeMismatch(mismatch) if mismatch.pos.is_none() => mismatch.pos = Some(pos),
        _ => {}
    }
    error
}

/// Record the position of the alias reference through which the current node
/// was reached. The position of the (definition) node itself is kept as the
/// primary position of the error; the alias reference is attached alongside
/// it so that both can be reported.
pub(crate) fn fix_alias_mark(mut error: Error, mark: libyaml::Mark) -> Error {
    let pos = match error.0.as_mut() {
        ErrorImpl::Message(_, pos) => pos,
        ErrorImpl::TypeMismatch(mismatch) => &mut mismatch.pos,
        _ => return error,
    };
    if let Some(pos) = pos {
        if pos.alias.is_none() {
            pos.alias = Some(mark);
        }
    }
    error
}

/// Turn an `invalid_type` error produced by serde into an explicit
/// `TypeMismatch` that records the expected type and the kind of YAML node
/// that was found, keeping the original message unchanged.
pub(crate) fn type_mismatch(err: Error, expected: String, found: NodeKind) -> Error {
    match *err.0 {
        ErrorImpl::Message(msg, None) => Error(Box::new(ErrorImpl::TypeMismatch(Box::new(
            TypeMismatchImpl {
                msg,
                expected,
                found,
                pos: None,
            },
        )))),
        other => Error(Box::new(other)),
    }
}

impl Error {
    pub(crate) fn shared(self) -> Arc<ErrorImpl> {
        if let ErrorImpl::Shared(err) = *self.0 {
            err
        } else {
            Arc::from(self.0)
        }
    }
}

impl From<libyaml::Error> for Error {
    fn from(err: libyaml::Error) -> Self {
        Error(Box::new(ErrorImpl::Libyaml(err)))
    }
}

impl From<emitter::Error> for Error {
    fn from(err: emitter::Error) -> Self {
        match err {
            emitter::Error::Libyaml(err) => Self::from(err),
            emitter::Error::Io(err) => new(ErrorImpl::Io(err)),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.display(f)
    }
}

// Remove two layers of verbosity from the debug representation. Humans often
// end up seeing this representation because it is what unwrap() shows.
impl Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.debug(f)
    }
}

impl ser::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Error(Box::new(ErrorImpl::Message(msg.to_string(), None)))
    }
}

impl de::Error for Error {
    fn custom<T: Display>(msg: T) -> Self {
        Error(Box::new(ErrorImpl::Message(msg.to_string(), None)))
    }
}

impl ErrorImpl {
    fn location(&self) -> Option<Location> {
        self.mark().map(Location::from_mark)
    }

    fn type_source(&self) -> Option<TypeSource> {
        match self {
            ErrorImpl::TypeMismatch(mismatch) => Some(TypeSource {
                expected: mismatch.expected.clone(),
                found: mismatch.found,
                mark: mismatch.pos.as_ref().map(|pos| pos.mark),
                alias_reference: mismatch.pos.as_ref().and_then(|pos| pos.alias),
            }),
            ErrorImpl::Shared(err) => err.type_source(),
            _ => None,
        }
    }

    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ErrorImpl::Io(err) => err.source(),
            ErrorImpl::FromUtf8(err) => err.source(),
            ErrorImpl::Shared(err) => err.source(),
            _ => None,
        }
    }

    fn mark(&self) -> Option<libyaml::Mark> {
        match self {
            ErrorImpl::Message(_, Some(Pos { mark, .. }))
            | ErrorImpl::RecursionLimitExceeded(mark)
            | ErrorImpl::UnknownAnchor(mark) => Some(*mark),
            ErrorImpl::TypeMismatch(mismatch) => mismatch.pos.as_ref().map(|pos| pos.mark),
            ErrorImpl::Libyaml(err) => Some(err.mark()),
            ErrorImpl::Shared(err) => err.mark(),
            _ => None,
        }
    }

    fn alias_mark(&self) -> Option<libyaml::Mark> {
        match self {
            ErrorImpl::Message(_, Some(pos)) => pos.alias,
            ErrorImpl::TypeMismatch(mismatch) => mismatch.pos.as_ref().and_then(|pos| pos.alias),
            ErrorImpl::Shared(err) => err.alias_mark(),
            _ => None,
        }
    }

    fn message_no_mark(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn write_path(f: &mut fmt::Formatter, pos: &Option<Pos>) -> fmt::Result {
            if let Some(Pos { path, .. }) = pos {
                if path != "." {
                    write!(f, "{}: ", path)?;
                }
            }
            Ok(())
        }

        match self {
            ErrorImpl::Message(msg, pos) => {
                write_path(f, pos)?;
                f.write_str(msg)
            }
            ErrorImpl::TypeMismatch(mismatch) => {
                write_path(f, &mismatch.pos)?;
                f.write_str(&mismatch.msg)
            }
            ErrorImpl::Libyaml(_) => unreachable!(),
            ErrorImpl::Io(err) => Display::fmt(err, f),
            ErrorImpl::FromUtf8(err) => Display::fmt(err, f),
            ErrorImpl::EndOfStream => f.write_str("EOF while parsing a value"),
            ErrorImpl::MoreThanOneDocument => f.write_str(
                "deserializing from YAML containing more than one document is not supported",
            ),
            ErrorImpl::RecursionLimitExceeded(_mark) => f.write_str("recursion limit exceeded"),
            ErrorImpl::RepetitionLimitExceeded => f.write_str("repetition limit exceeded"),
            ErrorImpl::BytesUnsupported => {
                f.write_str("serialization and deserialization of bytes in YAML is not implemented")
            }
            ErrorImpl::UnknownAnchor(_mark) => f.write_str("unknown anchor"),
            ErrorImpl::SerializeNestedEnum => {
                f.write_str("serializing nested enums in YAML is not supported yet")
            }
            ErrorImpl::ScalarInMerge => {
                f.write_str("expected a mapping or list of mappings for merging, but found scalar")
            }
            ErrorImpl::TaggedInMerge => f.write_str("unexpected tagged value in merge"),
            ErrorImpl::ScalarInMergeElement => {
                f.write_str("expected a mapping for merging, but found scalar")
            }
            ErrorImpl::SequenceInMergeElement => {
                f.write_str("expected a mapping for merging, but found sequence")
            }
            ErrorImpl::EmptyTag => f.write_str("empty YAML tag is not allowed"),
            ErrorImpl::FailedToParseNumber => f.write_str("failed to parse YAML number"),
            ErrorImpl::Shared(_) => unreachable!(),
        }
    }

    fn display(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorImpl::Libyaml(err) => Display::fmt(err, f),
            ErrorImpl::Shared(err) => err.display(f),
            _ => {
                self.message_no_mark(f)?;
                if let Some(mark) = self.mark() {
                    if mark.line() != 0 || mark.column() != 0 {
                        write!(f, " at {}", mark)?;
                    }
                }
                if let Some(alias) = self.alias_mark() {
                    if alias.line() != 0 || alias.column() != 0 {
                        write!(f, " (referenced by alias at {})", alias)?;
                    }
                }
                Ok(())
            }
        }
    }

    fn debug(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorImpl::Libyaml(err) => Debug::fmt(err, f),
            ErrorImpl::Shared(err) => err.debug(f),
            _ => {
                f.write_str("Error(")?;
                struct MessageNoMark<'a>(&'a ErrorImpl);
                impl<'a> Display for MessageNoMark<'a> {
                    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        self.0.message_no_mark(f)
                    }
                }
                let msg = MessageNoMark(self).to_string();
                Debug::fmt(&msg, f)?;
                if let Some(mark) = self.mark() {
                    write!(
                        f,
                        ", line: {}, column: {}",
                        mark.line() + 1,
                        mark.column() + 1,
                    )?;
                }
                if let Some(alias) = self.alias_mark() {
                    write!(
                        f,
                        ", alias line: {}, column: {}",
                        alias.line() + 1,
                        alias.column() + 1,
                    )?;
                }
                f.write_str(")")
            }
        }
    }
}
