//! ESC/POS byte emitter, Epson dialect.
//!
//! Command bytes are tabulated in `INSTRUCTIONS.md` §6, sourced from the Epson ESC/POS
//! Command Reference. When extending this, use that reference — the blog posts are
//! wrong about `GS !` nibble order often enough to matter.
//!
//! Bear in mind "standard ESC/POS" is not a standard: it is Epson's proprietary command
//! set, copied to varying degrees by vendors with no spec body and no certification.
//! Clone deviations cluster in cut variants, `ESC $` handling, and native QR. That is
//! unfixable in principle, which is what [`Op::Raw`] exists for.

use crate::ir::{Align, CutKind, Op};
use crate::profile::{CodePage, Font, Profile};
use crate::Error;

// --- Command bytes (INSTRUCTIONS.md §6) -------------------------------------------

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const LF: u8 = 0x0A;

/// `ESC @` — initialize, resetting all device state.
const INIT: [u8; 2] = [ESC, 0x40];

/// Emit a complete, self-contained ESC/POS document.
///
/// The stream is framed here: `ESC @` first, then font and code page selection, then
/// the ops. Framing is deliberately *not* in the IR — init has no meaning in the
/// preview backend, and putting it in the op stream would let a template forge it.
///
/// The trailing feed and cut come from [`layout`](crate::layout), which guarantees them
/// whether or not the template asked.
pub fn emit(ops: &[Op], profile: &Profile) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(ops.len() * 8 + 16);

    out.extend_from_slice(&INIT);
    // ESC M n — select font.
    out.extend_from_slice(&[
        ESC,
        0x4D,
        match profile.font {
            Font::A => 0,
            Font::B => 1,
        },
    ]);
    // ESC t n — select code page. Page 0 is CP437 on essentially every device.
    out.extend_from_slice(&[
        ESC,
        0x74,
        match profile.code_page {
            CodePage::Cp437 => 0,
        },
    ]);

    for op in ops {
        emit_op(op, profile, &mut out)?;
    }

    Ok(out)
}

fn emit_op(op: &Op, profile: &Profile, out: &mut Vec<u8>) -> Result<(), Error> {
    match op {
        Op::Text(s) => encode_text(s, profile.code_page, out)?,

        // ESC E n
        Op::Emphasis(on) => out.extend_from_slice(&[ESC, 0x45, u8::from(*on)]),

        // ESC - n
        Op::Underline(on) => out.extend_from_slice(&[ESC, 0x2D, u8::from(*on)]),

        // ESC a n
        Op::Justify(a) => out.extend_from_slice(&[
            ESC,
            0x61,
            match a {
                Align::Left => 0,
                Align::Center => 1,
                Align::Right => 2,
            },
        ]),

        // GS ! n — high nibble is width-1, low nibble is height-1.
        Op::Size { w, h } => {
            let n = ((w.saturating_sub(1) & 0x07) << 4) | (h.saturating_sub(1) & 0x07);
            out.extend_from_slice(&[GS, 0x21, n]);
        }

        // ESC $ nL nH — dots from the left margin.
        Op::AbsPos(dots) => {
            out.extend_from_slice(&[ESC, 0x24, (dots & 0xFF) as u8, (dots >> 8) as u8]);
        }

        // ESC d n
        Op::Feed(n) => out.extend_from_slice(&[ESC, 0x64, *n]),

        Op::Cut(kind) => {
            // GS V 66 0 is the feed-and-cut variant: it advances the paper past the
            // cutter before cutting, so the last line is not sliced through. Falling
            // back to a full cut when the profile says partial is unsupported is
            // better than emitting a command the mechanism will ignore.
            match kind {
                CutKind::Partial if profile.supports_partial_cut => {
                    out.extend_from_slice(&[GS, 0x56, 0x42, 0x00]);
                }
                _ => out.extend_from_slice(&[GS, 0x56, 0x00]),
            }
        }

        Op::Raw(bytes) => out.extend_from_slice(bytes),
    }
    Ok(())
}

/// Encode text into the profile's code page.
///
/// v0.1 covers ASCII and the newline. The CP437 high range (0x80..=0xFF: accented Latin,
/// box drawing) still needs its mapping table — until then, characters outside ASCII are
/// rejected rather than silently replaced. A `?` on a printed receipt is a support call
/// that never traces back to this function.
fn encode_text(s: &str, code_page: CodePage, out: &mut Vec<u8>) -> Result<(), Error> {
    match code_page {
        CodePage::Cp437 => {
            for ch in s.chars() {
                match ch {
                    '\n' => out.push(LF),
                    c if c.is_ascii() => out.push(c as u8),
                    c => return Err(Error::Unrepresentable { ch: c }),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops_only(ops: &[Op]) -> Vec<u8> {
        // Strip the 8-byte frame (ESC @, ESC M n, ESC t n) to assert on op bytes alone.
        emit(ops, &Profile::epson_80mm()).unwrap()[8..].to_vec()
    }

    #[test]
    fn document_is_framed_with_init() {
        let bytes = emit(&[], &Profile::epson_80mm()).unwrap();
        assert_eq!(&bytes[..2], &[0x1B, 0x40]);
    }

    #[test]
    fn size_packs_nibbles_width_high_height_low() {
        // 2x2 is 0x11, per INSTRUCTIONS.md §6.
        assert_eq!(ops_only(&[Op::Size { w: 2, h: 2 }]), vec![0x1D, 0x21, 0x11]);
        assert_eq!(ops_only(&[Op::Size { w: 1, h: 1 }]), vec![0x1D, 0x21, 0x00]);
        // Width 3, height 1 — high nibble only.
        assert_eq!(ops_only(&[Op::Size { w: 3, h: 1 }]), vec![0x1D, 0x21, 0x20]);
    }

    #[test]
    fn abspos_is_little_endian() {
        // 576 dots = 0x0240 -> nL 0x40, nH 0x02.
        assert_eq!(ops_only(&[Op::AbsPos(576)]), vec![0x1B, 0x24, 0x40, 0x02]);
        assert_eq!(ops_only(&[Op::AbsPos(0)]), vec![0x1B, 0x24, 0x00, 0x00]);
    }

    #[test]
    fn partial_cut_falls_back_when_unsupported() {
        let capable = Profile::epson_80mm();
        assert_eq!(
            ops_only(&[Op::Cut(CutKind::Partial)]),
            vec![0x1D, 0x56, 0x42, 0x00]
        );

        let incapable = Profile {
            supports_partial_cut: false,
            ..capable
        };
        let bytes = emit(&[Op::Cut(CutKind::Partial)], &incapable).unwrap();
        assert_eq!(&bytes[8..], &[0x1D, 0x56, 0x00]);
    }

    #[test]
    fn raw_passes_through_untouched() {
        let quirk = vec![0x1D, 0x56, 0x41, 0x10];
        assert_eq!(ops_only(&[Op::Raw(quirk.clone())]), quirk);
    }

    #[test]
    fn non_ascii_is_rejected_not_replaced() {
        let err = emit(&[Op::Text("café".into())], &Profile::epson_80mm()).unwrap_err();
        assert_eq!(err, Error::Unrepresentable { ch: 'é' });
    }
}
