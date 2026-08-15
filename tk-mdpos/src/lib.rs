//! Turn a formatted template string into ESC/POS bytes.
//!
//! ```text
//! (template: &str, profile: &Profile) -> Result<Vec<u8>, Error>
//! ```
//!
//! That is the entire public contract. Layout lives in a *string* — one that can sit in
//! a database row or a config field — so changing a receipt footer is a data edit rather
//! than a rebuild, a redeploy, and a test cycle. Every other ESC/POS library is a command
//! builder that compiles layout into the application.
//!
//! # Sans-IO
//!
//! This crate does not know what a printer is. No sockets, no serial ports, no USB, no
//! filesystem, no async runtime. It hands back bytes; delivering them is the caller's
//! problem, and deliberately so — printer transport is a platform tarpit, and the largest
//! target hardware exposes nothing but `sendRAWData(byte[])` through a vendor service
//! anyway. Queueing, chunking, retry, status polling, and job atomicity are all out of
//! scope.
//!
//! # Pipeline
//!
//! ```text
//! template string
//!       ↓  parse      cheap, syntax only
//!    Block AST
//!       ↓  layout     THE PRODUCT — consumes the Profile
//!    Vec<Op>          device-independent IR, width-resolved
//!       ↓  emit       per-backend, nearly mechanical
//!    Vec<u8>
//! ```
//!
//! The IR in the middle is not optional (`INSTRUCTIONS.md` §1.2). It is what makes the
//! preview backend, additional dialects, and a future vendor-SDK backend possible at all,
//! and it costs about ten lines of discipline to maintain.
//!
//! # Format versioning
//!
//! Templates may open with `{v 1}`. The *string* carries the compatibility promise, not
//! the crate version: the engine may be rewritten freely, but a v1 template must render
//! identically in perpetuity. If syntax changes could drag deployed templates back into
//! the redeploy cycle, the premise of the library collapses.

pub mod emit;
pub mod error;
pub mod ir;
pub mod layout;
pub mod parse;
pub mod profile;
pub mod qr;

pub use error::Error;
pub use ir::{Align, CutKind, Op};
pub use profile::{CodePage, Dialect, Font, Profile};

/// Render a template to ESC/POS bytes.
///
/// The result is self-contained: it begins with `ESC @` and ends with a feed and a cut,
/// and assumes nothing about the device's prior state. A thermal printer is a stateful
/// interpreter — leave emphasis on and every subsequent receipt prints bold until someone
/// power-cycles it. Self-containment means a lost or duplicated document cannot corrupt
/// the next one.
pub fn render(template: &str, profile: &Profile) -> Result<Vec<u8>, Error> {
    let ast = parse::parse(template)?;
    let ops = layout::layout(&ast, profile)?;
    emit::escpos::emit(&ops, profile)
}

/// Render a template to monospace plaintext, for previewing layout without a printer.
///
/// Shares the parse and layout passes with [`render`], which is the point — a preview
/// produced by a separate code path would drift from the bytes and quietly stop being
/// evidence of anything.
pub fn preview(template: &str, profile: &Profile) -> Result<String, Error> {
    let ast = parse::parse(template)?;
    let ops = layout::layout(&ast, profile)?;
    emit::preview::emit(&ops, profile)
}

/// Render a template to the intermediate representation.
///
/// Exposed for tests and tooling. Most callers want [`render`] or [`preview`].
pub fn to_ops(template: &str, profile: &Profile) -> Result<Vec<Op>, Error> {
    let ast = parse::parse(template)?;
    layout::layout(&ast, profile)
}
