//! The device-independent intermediate representation.
//!
//! Everything upstream of this module knows about templates; everything downstream
//! knows about printers. Nothing knows about both.
//!
//! `Op` is **post-layout**: by the time a stream of these exists, every width question
//! has been answered against a [`Profile`](crate::Profile). Note the absence of any
//! `Row` or `Cell` variant — columns resolve during layout into [`Op::Text`] and
//! [`Op::AbsPos`] pairs. If a cell concept starts leaking in here, the layout pass is
//! incomplete; fix it there rather than widening the IR.

/// A single device-independent rendering instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Literal text, already width-resolved and broken into lines by layout.
    ///
    /// There is no separate line-break op: an embedded `\n` is the line break, and
    /// emitters map it to `LF` (which is what commits the printer's line buffer to
    /// paper). Layout owns every decision about where those breaks fall.
    Text(String),
    /// Bold on/off.
    Emphasis(bool),
    /// Underline on/off.
    Underline(bool),
    /// Justification. Sticky on the device until changed.
    Justify(Align),
    /// Character magnification, 1..=8 in each axis.
    Size { w: u8, h: u8 },
    /// Absolute horizontal position, in dots from the left margin.
    ///
    /// This is how right-alignment is done — see the module docs on `layout`.
    /// Never emit space padding to position text.
    AbsPos(u16),
    /// Advance N lines.
    Feed(u8),
    /// Cut the paper.
    Cut(CutKind),
    /// Raw byte passthrough. The escape hatch for clone-printer quirks; see
    /// `INSTRUCTIONS.md` §7 for why this is load-bearing rather than a hack.
    Raw(Vec<u8>),
    // v0.2+: Qr { .. }, Barcode { .. }, Image { .. }
}

/// Horizontal justification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Which cut a [`Op::Cut`] requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CutKind {
    /// Leaves a small tab of paper. The safe default.
    #[default]
    Partial,
    /// Severs completely. Not all mechanisms support this.
    Full,
}
