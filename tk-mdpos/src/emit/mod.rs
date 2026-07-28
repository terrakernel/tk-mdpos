//! Backends. Each turns the same [`Op`](crate::Op) stream into one concrete output.
//!
//! Emitters are nearly mechanical by design: all the judgment happened in
//! [`layout`](crate::layout). An emitter that needs to make a width decision is a sign
//! that something belongs upstream.
//!
//! Both backends snapshot from the same golden fixture, which is what keeps the preview
//! honest about what the bytes will actually do.

pub mod escpos;
pub mod preview;
