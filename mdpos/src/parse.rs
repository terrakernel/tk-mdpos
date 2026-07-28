//! Template text into a block AST. The cheap part — roughly 300 lines when done.
//!
//! The parser resolves *syntax* only. It must not consult the [`Profile`] or compute a
//! single width: `{cols 20,10:r}` becomes a spec of two columns in characters, and what
//! that means in dots is entirely the layout pass's problem.
//!
//! Status: **not implemented**. Types below are the intended shape.

use crate::ir::Align;

/// A parsed block with the source line it came from, for error reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// 1-based source line.
    pub line: usize,
    pub block: Block,
}

/// One structural element of a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `{v N}` — format version. Optional in v0.1, assumed 1.
    Version(u32),
    /// `{left}` `{center}` `{right}` — sticky until changed.
    Justify(Align),
    /// `{size WxH}` — magnification, 1..=8 each. Sticky.
    Size { w: u8, h: u8 },
    /// `{cols A,B:r,C:c}` — widths in *characters*. Sticky until `{/cols}` or the next
    /// `{cols}`.
    Cols(Vec<ColSpec>),
    /// `{/cols}` — leave column mode.
    ColsEnd,
    /// `---` — full-width separator.
    Rule,
    /// `{feed N}`.
    Feed(u8),
    /// `{cut}`.
    Cut,
    /// `{raw 1D564200}` — already hex-decoded.
    Raw(Vec<u8>),
    /// A content line. One cell when no `{cols}` is active, otherwise split on
    /// unescaped `|`.
    Line(Vec<Cell>),
}

/// One column of an active `{cols}` spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColSpec {
    /// Width in characters at magnification 1x.
    pub width_chars: u16,
    /// `:l` (default), `:r`, or `:c`.
    pub align: Align,
}

/// The contents of one cell: a sequence of attributed text runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub spans: Vec<Span>,
}

/// A run of text sharing one set of inline attributes.
///
/// Inline markup is a deliberately tiny markdown subset — `**bold**` and `__underline__`
/// and nothing else. Attributes are flattened into runs here rather than kept as a tree,
/// because layout measures widths run by run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub underline: bool,
}

/// The format version assumed when a template omits `{v N}`.
pub const DEFAULT_VERSION: u32 = 1;

/// The highest format version this build implements.
///
/// Raising this is a promise: every template at or below it must render identically
/// forever after (`INSTRUCTIONS.md` §1.3).
pub const MAX_VERSION: u32 = 1;

/// Parse template text into a block AST.
pub fn parse(_template: &str) -> Result<Vec<Node>, crate::Error> {
    todo!("parser — see INSTRUCTIONS.md §4 for the syntax table")
}
