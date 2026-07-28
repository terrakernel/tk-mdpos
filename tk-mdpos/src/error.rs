//! Rendering errors.
//!
//! Errors carry a 1-based line number where one is known. Template authors are editing
//! a string in a database field or a text area, often without a compiler between them
//! and the printer — a message that does not say *which line* is nearly useless.

use std::fmt;

/// Anything that can go wrong turning a template into bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A `{...}` directive was not recognized.
    UnknownDirective { line: usize, name: String },
    /// A directive was recognized but its arguments did not parse.
    BadDirective {
        line: usize,
        name: String,
        detail: String,
    },
    /// `{v N}` named a format version this build does not implement.
    ///
    /// v1 templates must render identically in perpetuity (`INSTRUCTIONS.md` §1.3), so
    /// this is only ever reported for versions from the future, never the past.
    UnsupportedVersion { line: usize, requested: u32 },
    /// A `{size WxH}` outside 1..=8.
    SizeOutOfRange { line: usize, w: u8, h: u8 },
    /// A row had a different number of cells than the active `{cols}` spec.
    ColumnCountMismatch {
        line: usize,
        expected: usize,
        found: usize,
    },
    /// A `{cols}` spec is wider than the paper at current magnification.
    ColumnsTooWide {
        line: usize,
        requested: u16,
        available: u16,
    },
    /// Content overflowed a right-aligned column, which must never wrap.
    ///
    /// A wrapped total is worse than a rejected template, so this is an error rather
    /// than a silent truncation (`INSTRUCTIONS.md` §5.4).
    RightColumnOverflow {
        line: usize,
        column: usize,
        width: u16,
        content: String,
    },
    /// `{raw ...}` payload was not valid hex, or had odd length.
    BadHex { line: usize, detail: String },
    /// A character has no representation in the profile's code page.
    Unrepresentable { ch: char },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownDirective { line, name } => {
                write!(f, "line {line}: unknown directive `{{{name}}}`")
            }
            Error::BadDirective { line, name, detail } => {
                write!(f, "line {line}: bad `{{{name}}}` directive: {detail}")
            }
            Error::UnsupportedVersion { line, requested } => write!(
                f,
                "line {line}: template format version {requested} is newer than this build supports"
            ),
            Error::SizeOutOfRange { line, w, h } => {
                write!(f, "line {line}: size {w}x{h} outside the supported range 1..=8")
            }
            Error::ColumnCountMismatch {
                line,
                expected,
                found,
            } => write!(
                f,
                "line {line}: active {{cols}} spec has {expected} columns but the row has {found}"
            ),
            Error::ColumnsTooWide {
                line,
                requested,
                available,
            } => write!(
                f,
                "line {line}: columns total {requested} chars but only {available} fit at the current size"
            ),
            Error::RightColumnOverflow {
                line,
                column,
                width,
                content,
            } => write!(
                f,
                "line {line}: {content:?} overflows right-aligned column {column} (width {width}); \
                 right-aligned columns never wrap"
            ),
            Error::BadHex { line, detail } => {
                write!(f, "line {line}: bad `{{raw}}` payload: {detail}")
            }
            Error::Unrepresentable { ch } => {
                write!(f, "character {ch:?} has no encoding in the target code page")
            }
        }
    }
}

impl std::error::Error for Error {}
