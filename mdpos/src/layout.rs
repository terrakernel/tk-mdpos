//! Block AST + [`Profile`] into a flat [`Op`] stream. **This is the product.**
//!
//! Every hard problem in this crate is in this file. The parser is mechanical and the
//! emitters are nearly so; if this pass is good, the library is good.
//!
//! Status: **not implemented**. What follows is the specification it has to satisfy —
//! see `INSTRUCTIONS.md` §5 for the reasoning behind each point.
//!
//! # Invariants
//!
//! **Right-alignment is arithmetic in dots.** Compute a target column position and emit
//! [`Op::AbsPos`]. Never pad with spaces: padding is correct only while the character
//! cell width is constant, and it stops being constant the moment magnification or the
//! font changes mid-line.
//!
//! **Magnification mutates the grid.** `{size 2x2}` takes 48 columns to 24. Track the
//! *current* character width through the whole pass; a document-level constant is the
//! most common source of output that looks almost right. Use
//! [`Profile::columns_at`](crate::Profile::columns_at).
//!
//! **Width is [`unicode_width`], always.** Not `str::len()`, not `chars().count()`.
//! Both are wrong for the CJK and combining cases that customer names eventually
//! deliver, and being mostly-ASCII in practice is not a reason to be wrong in principle.
//!
//! **Overflow policy is per-column.** `:l` and `:c` wrap with a hanging indent, with
//! continuation lines positioned at the column's own start. `:r` never wraps — it
//! raises [`Error::RightColumnOverflow`](crate::Error::RightColumnOverflow), because a
//! wrapped total is worse than a rejected template.
//!
//! **The output is self-contained.** The stream this pass produces must end with a feed
//! and a cut whether or not the template asked. The printer is a stateful interpreter,
//! and a document that leaves state dirty corrupts the *next* receipt — possibly on
//! someone else's order. (The matching `ESC @` init is the emitter's job, since it is a
//! byte-level concern with no meaning in the preview backend.)
//!
//! # Column resolution
//!
//! `{cols}` disappears here. A row becomes, per cell: an [`Op::AbsPos`] to the cell's
//! start (or its right-aligned offset), then the cell's [`Op::Text`]. Rows that wrap
//! emit several such groups separated by newlines. No cell concept survives into the IR.

use crate::ir::Op;
use crate::parse::Node;
use crate::profile::Profile;

/// Lay out a parsed template against a profile, producing the device-independent IR.
pub fn layout(_ast: &[Node], _profile: &Profile) -> Result<Vec<Op>, crate::Error> {
    todo!("layout engine — the whole product; see the module docs and INSTRUCTIONS.md §5")
}
