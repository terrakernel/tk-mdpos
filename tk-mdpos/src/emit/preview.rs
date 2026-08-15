//! Monospace plaintext backend.
//!
//! A preview that can silently diverge from print is worse than no preview, which is the
//! reason a browser-and-CSS preview was rejected (`INSTRUCTIONS.md` §8). Monospace text
//! is honest about the one thing that matters: the fixed-width grid.
//!
//! It cannot show bold, underline, or magnification — a 2x character occupies two cells
//! on paper and one in a terminal. So magnification is tracked for *positioning* only:
//! [`Op::AbsPos`] resolves to the character column it will land on, since that is what
//! layout bugs actually look like.

use crate::ir::{Align, CutKind, Op};
use crate::profile::Profile;
use crate::Error;

/// Render an op stream as monospace text.
pub fn emit(ops: &[Op], profile: &Profile) -> Result<String, Error> {
    let cell_dots = profile.font.char_width_dots();
    let cols = profile.columns() as usize;

    let mut out = String::new();
    let mut line = String::new();
    let mut justify = Align::Left;
    // Magnification is not rendered, but it does change how wide a line may be.
    let mut mag_w: u8 = 1;

    for op in ops {
        match op {
            Op::Text(s) => {
                for ch in s.chars() {
                    if ch == '\n' {
                        // An explicit break always commits a line, even an empty one.
                        commit(&mut out, &mut line, justify, cols, mag_w);
                    } else {
                        line.push(ch);
                    }
                }
            }

            Op::AbsPos(dots) => {
                // Pad to the target column. If the line is already past it, layout has
                // overflowed a cell — leave it visibly wrong rather than reflowing.
                let target = (*dots / cell_dots) as usize;
                let current = width_of(&line);
                if target > current {
                    line.extend(std::iter::repeat_n(' ', target - current));
                }
            }

            Op::Justify(a) => justify = *a,
            Op::Size { w, .. } => mag_w = (*w).max(1),

            Op::Feed(n) => {
                flush_pending(&mut out, &mut line, justify, cols, mag_w);
                for _ in 0..*n {
                    out.push('\n');
                }
            }

            Op::Cut(kind) => {
                flush_pending(&mut out, &mut line, justify, cols, mag_w);
                let mark = match kind {
                    CutKind::Partial => '-',
                    CutKind::Full => '=',
                };
                out.extend(std::iter::repeat_n(mark, cols));
                out.push('\n');
            }

            // Nothing to show, but they must not silently vanish from a diff either.
            Op::Emphasis(_) | Op::Underline(_) => {}
            Op::Raw(bytes) => {
                flush_pending(&mut out, &mut line, justify, cols, mag_w);
                out.push_str(&format!("<raw {} bytes>\n", bytes.len()));
            }

            Op::Qr { data, module } => {
                flush_pending(&mut out, &mut line, justify, cols, mag_w);
                // Layout already proved the symbol fits, so the footprint is known good;
                // the fallback only keeps this backend total.
                let dots = crate::qr::footprint_dots(data.len(), *module).unwrap_or(0);
                let width = dots.div_ceil(cell_dots) as usize;
                draw_qr(&mut out, data, width, justify, cols);
            }
        }
    }

    flush_pending(&mut out, &mut line, justify, cols, mag_w);
    Ok(out)
}

/// Commit the pending line, applying justification across the current grid.
fn commit(out: &mut String, line: &mut String, justify: Align, cols: usize, mag_w: u8) {
    if line.is_empty() {
        // A blank line is still a line. Justifying it would emit trailing spaces.
        out.push('\n');
        return;
    }

    // Everything here is measured in *base* cells, the same units `Op::AbsPos` resolves
    // to, so the two never disagree about where column 20 is.
    //
    // A magnified character occupies `mag_w` base cells on paper even though it occupies
    // one cell in a terminal, so the text is measured at its printed width and then drawn
    // narrow. The line therefore starts where it really starts and ends early — position
    // is what layout bugs corrupt, and position is what this has to get right.
    let printed = width_of(line) * usize::from(mag_w.max(1));
    let pad = cols.saturating_sub(printed);

    match justify {
        Align::Left => {}
        Align::Center => out.extend(std::iter::repeat_n(' ', pad / 2)),
        Align::Right => out.extend(std::iter::repeat_n(' ', pad)),
    }
    out.push_str(line);
    out.push('\n');
    line.clear();
}

/// Commit a line only if one is actually pending.
///
/// Used where an op implies a line boundary without being one — end of stream, or before
/// a feed or cut. Committing unconditionally there appends a phantom blank line, which
/// then bakes itself into every golden fixture.
fn flush_pending(out: &mut String, line: &mut String, justify: Align, cols: usize, mag_w: u8) {
    if !line.is_empty() {
        commit(out, line, justify, cols, mag_w);
    }
}

/// Sketch a QR symbol as a box of its true printed width.
///
/// Only the width and the horizontal placement are real. The box is three lines tall
/// regardless of the symbol's actual height, because height is not a thing layout can get
/// wrong — nothing shares the line with a QR. Width is, so width is to scale, and the
/// payload is shown so a fixture diff says which code moved.
fn draw_qr(out: &mut String, data: &str, width: usize, justify: Align, cols: usize) {
    let pad = match justify {
        Align::Left => 0,
        Align::Center => cols.saturating_sub(width) / 2,
        Align::Right => cols.saturating_sub(width),
    };

    let mut row = |content: String| {
        out.extend(std::iter::repeat_n(' ', pad));
        out.push_str(&content);
        out.push('\n');
    };

    // Too narrow to caption — a version-1 symbol at module size 1 is barely two cells.
    if width < 4 {
        row("#".repeat(width));
        return;
    }

    let inner = width - 2;
    let mut caption = String::from("QR ");
    caption.push_str(data);
    let caption: String = if width_of(&caption) > inner {
        // Truncate by display width, so a wide character never splits the border.
        let mut taken = String::new();
        for ch in caption.chars() {
            if width_of(&taken) + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) > inner {
                break;
            }
            taken.push(ch);
        }
        taken
    } else {
        caption
    };

    let border = format!("+{}+", "-".repeat(inner));
    row(border.clone());
    row(format!(
        "|{caption}{}|",
        " ".repeat(inner - width_of(&caption))
    ));
    row(border);
}

/// Display width, per `unicode-width`. Never `len()`, never `chars().count()`.
fn width_of(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abspos_pads_to_the_character_column() {
        let p = Profile::epson_80mm();
        // Font A is 12 dots per cell, so 120 dots is column 10.
        let ops = vec![
            Op::Text("AB".into()),
            Op::AbsPos(120),
            Op::Text("C\n".into()),
        ];
        assert_eq!(emit(&ops, &p).unwrap(), "AB        C\n");
    }

    #[test]
    fn centering_accounts_for_the_printed_width_of_magnified_text() {
        let p = Profile::epson_80mm();

        let normal = emit(&[Op::Justify(Align::Center), Op::Text("HI\n".into())], &p).unwrap();
        assert_eq!(normal, format!("{}HI\n", " ".repeat(23)));

        // At 2x, "HI" covers 4 base cells on paper, so the device centers it at cell 22.
        // Predicting cell 11 would be measuring in magnified cells while `AbsPos` is
        // measured in base ones — two coordinate systems in one preview.
        let doubled = emit(
            &[
                Op::Justify(Align::Center),
                Op::Size { w: 2, h: 2 },
                Op::Text("HI\n".into()),
            ],
            &p,
        )
        .unwrap();
        assert_eq!(doubled, format!("{}HI\n", " ".repeat(22)));
    }

    #[test]
    fn right_align_pads_to_the_margin() {
        let p = Profile::epson_80mm();
        let out = emit(&[Op::Justify(Align::Right), Op::Text("END\n".into())], &p).unwrap();
        assert_eq!(out, format!("{}END\n", " ".repeat(45)));
    }

    #[test]
    fn qr_box_is_drawn_at_the_symbols_true_width() {
        let p = Profile::epson_80mm();
        // "HI" is 2 bytes -> version 1 -> 21 modules + 8 quiet = 29, at module 6 = 174
        // dots, which is 14.5 cells and so rounds up to 15.
        let out = emit(
            &[Op::Qr {
                data: "HI".into(),
                module: 6,
            }],
            &p,
        )
        .unwrap();

        let first = out.lines().next().unwrap();
        assert_eq!(first, "+-------------+");
        assert_eq!(width_of(first), 15);
        assert!(out.contains("|QR HI        |"));
    }

    #[test]
    fn a_centered_qr_is_offset_like_centered_text() {
        let p = Profile::epson_80mm();
        let out = emit(
            &[
                Op::Justify(Align::Center),
                Op::Qr {
                    data: "HI".into(),
                    module: 6,
                },
            ],
            &p,
        )
        .unwrap();

        // 48 cells, a 15-cell symbol, so 16 cells of lead on every line of the box.
        for line in out.lines() {
            assert_eq!(line.len() - line.trim_start().len(), 16, "{line:?}");
        }
    }

    #[test]
    fn a_long_payload_is_truncated_inside_the_border() {
        let p = Profile::epson_80mm();
        let out = emit(
            &[Op::Qr {
                data: "https://example.com/a/very/long/path/that/will/not/fit".into(),
                module: 3,
            }],
            &p,
        )
        .unwrap();

        // Every line of the box is the same width — a caption must never push the border
        // out and quietly misreport how wide the symbol is.
        let widths: Vec<usize> = out.lines().map(width_of).collect();
        assert_eq!(widths.len(), 3);
        assert!(widths.iter().all(|&w| w == widths[0]), "{widths:?}");
    }

    #[test]
    fn wide_characters_count_as_two_cells() {
        let p = Profile::epson_80mm();
        // Two CJK characters occupy four cells, so right-alignment pads 44, not 46.
        let out = emit(&[Op::Justify(Align::Right), Op::Text("日本\n".into())], &p).unwrap();
        assert_eq!(out, format!("{}日本\n", " ".repeat(44)));
    }
}
