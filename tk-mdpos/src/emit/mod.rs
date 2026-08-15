//! Backends. Each turns the same [`Op`](crate::Op) stream into one concrete output.
//!
//! Emitters are nearly mechanical by design: all the judgment happened in
//! [`layout`](crate::layout). An emitter that needs to make a width decision is a sign
//! that something belongs upstream.
//!
//! Every backend snapshots from the same golden fixture, which is what keeps the previews
//! honest about what the bytes will actually do.
//!
//! The two preview backends have different audiences and neither replaces the other:
//! [`preview`] is a developer's diff tool and is what the fixtures assert the grid
//! against, while [`html`] is for showing a person what the paper will look like.

pub mod escpos;
pub mod html;
pub mod preview;
