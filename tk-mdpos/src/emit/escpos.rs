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

        Op::Qr { data, module } => emit_qr(data, *module, out),
    }
    Ok(())
}

/// `GS ( k` — the two-dimensional symbol family.
///
/// Four calls, in this order: select model, set module size, set error correction, store
/// the payload, then print. Each is `GS ( k pL pH cn fn [parameters]`, where `pL + pH*256`
/// counts the bytes from `cn` onward and `cn = 49` selects the QR family.
///
/// The payload is written as UTF-8 rather than through the profile's code page. QR byte
/// mode carries opaque bytes and scanners decode them as UTF-8, so this is the one place
/// a non-ASCII character reaches the printer without [`encode_text`] having a say.
fn emit_qr(data: &str, module: u8, out: &mut Vec<u8>) {
    /// `cn` for the QR code family.
    const QR: u8 = 49;

    fn header(out: &mut Vec<u8>, body_len: usize) {
        out.extend_from_slice(&[GS, 0x28, 0x6B, (body_len & 0xFF) as u8, (body_len >> 8) as u8]);
    }

    // fn 65 — select model 2 (the universally supported one); the trailing 0 is unused.
    header(out, 4);
    out.extend_from_slice(&[QR, 65, 50, 0]);

    // fn 67 — module size in dots.
    header(out, 3);
    out.extend_from_slice(&[QR, 67, module]);

    // fn 69 — error correction. 48..=51 is L/M/Q/H; see `qr` for why M is fixed.
    header(out, 3);
    out.extend_from_slice(&[QR, 69, 49]);

    // fn 80 — store the payload in the symbol buffer. The body is cn, fn, m, then data.
    let bytes = data.as_bytes();
    header(out, bytes.len() + 3);
    out.extend_from_slice(&[QR, 80, 48]);
    out.extend_from_slice(bytes);

    // fn 81 — print what was stored. Advances the paper on its own, so no LF follows.
    header(out, 3);
    out.extend_from_slice(&[QR, 81, 48]);
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

    #[test]
    fn qr_emits_the_five_gs_paren_k_calls_in_order() {
        let bytes = ops_only(&[Op::Qr {
            data: "HI".into(),
            module: 6,
        }]);

        #[rustfmt::skip]
        let want = vec![
            0x1D, 0x28, 0x6B, 0x04, 0x00, 49, 65, 50, 0,  // model 2
            0x1D, 0x28, 0x6B, 0x03, 0x00, 49, 67, 6,      // module size 6
            0x1D, 0x28, 0x6B, 0x03, 0x00, 49, 69, 49,     // error correction M
            0x1D, 0x28, 0x6B, 0x05, 0x00, 49, 80, 48, b'H', b'I', // store "HI"
            0x1D, 0x28, 0x6B, 0x03, 0x00, 49, 81, 48,     // print
        ];
        assert_eq!(bytes, want);
    }

    #[test]
    fn qr_store_length_is_little_endian_across_the_256_byte_boundary() {
        // pL/pH count the payload plus cn, fn and m. At 253 bytes of data the body is
        // exactly 256, which is where a byte-order mistake stops being invisible.
        let bytes = ops_only(&[Op::Qr {
            data: "A".repeat(253),
            module: 4,
        }]);
        let store = bytes
            .windows(5)
            .find(|w| w[0] == 0x1D && w[1] == 0x28 && w[2] == 0x6B && w[3] == 0x00)
            .expect("a store call with pL == 0");
        assert_eq!(store[3], 0x00, "pL");
        assert_eq!(store[4], 0x01, "pH — 256 bytes, not 1");
    }

    #[test]
    fn qr_payload_bypasses_the_code_page() {
        // `é` has no CP437 mapping in this build and `Op::Text` rejects it, but a QR
        // carries opaque bytes, so the same character must survive as UTF-8 here.
        assert!(emit(&[Op::Text("é".into())], &Profile::epson_80mm()).is_err());

        let bytes = ops_only(&[Op::Qr {
            data: "é".into(),
            module: 6,
        }]);
        assert!(
            bytes.windows(2).any(|w| w == "é".as_bytes()),
            "payload should appear as UTF-8"
        );
    }
}
