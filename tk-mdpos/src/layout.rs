//! Block AST + [`Profile`] into a flat [`Op`] stream. **This is the product.**
//!
//! Every hard problem in this crate is in this file. The parser is mechanical and the
//! emitters are nearly so; if this pass is good, the library is good.
//!
//! # Invariants
//!
//! **Right-alignment is arithmetic in dots.** Column content is positioned with
//! [`Op::AbsPos`], never with space padding. Padding is correct only while the character
//! cell width is constant, and it stops being constant the moment magnification or the
//! font changes mid-line.
//!
//! **Magnification mutates the grid.** `{size 2x2}` takes 48 columns to 24, and a
//! `{cols}` width of 20 means 20 *current* characters — 40 base cells, 480 dots. Every
//! width question is asked against the current magnification, never a document constant.
//!
//! **Width is [`unicode_width`], always.** Not `str::len()`, not `chars().count()`.
//!
//! **Overflow policy is per-column.** `:l` and `:c` wrap, with continuation lines
//! repositioned to the column's own start — the hanging indent falls out of re-emitting
//! [`Op::AbsPos`] per line. `:r` never wraps; it raises
//! [`Error::RightColumnOverflow`], because a wrapped total is worse than a rejected
//! template.
//!
//! **The output is self-contained.** The stream always ends with a feed and a cut,
//! whether or not the template asked, with emphasis and underline turned back off. (The
//! matching `ESC @` is the emitter's job — it is a byte-level concern with no meaning in
//! the preview backend.)
//!
//! # Column resolution
//!
//! `{cols}` disappears here. A row becomes, per cell: an [`Op::AbsPos`] to the cell's
//! start plus its alignment offset, then the cell's [`Op::Text`]. Rows whose cells wrap
//! emit several such groups, one physical line each. No cell concept survives into the IR.

use unicode_width::UnicodeWidthChar;

use crate::ir::{Align, CutKind, Op};
use crate::parse::{Block, Cell, ColSpec, Node, Span};
use crate::profile::Profile;
use crate::qr;
use crate::Error;

/// Lines fed before the closing cut when the template does not end with one itself.
///
/// The cut command is the feed-and-cut variant, so this is not about clearing the
/// mechanism — it is so the last printed line is not flush against the tear edge.
const FINAL_FEED: u8 = 4;

/// Lay out a parsed template against a profile, producing the device-independent IR.
pub fn layout(ast: &[Node], profile: &Profile) -> Result<Vec<Op>, Error> {
    let mut engine = Engine::new(profile);
    engine.run(ast)?;
    Ok(engine.finish())
}

/// Inline attributes carried by a single character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Attrs {
    bold: bool,
    underline: bool,
}

/// A character paired with the attributes it was written under.
///
/// Wrapping happens over this flattened form rather than over [`Span`]s, because a line
/// break can land in the middle of a span and splitting spans mid-wrap is where the
/// fiddly bugs live. Spans are rebuilt per output line at the end.
type Glyph = (char, Attrs);

struct Engine<'a> {
    profile: &'a Profile,
    ops: Vec<Op>,

    // Sticky template state.
    justify: Align,
    mag: (u8, u8),
    cols: Option<Vec<ColSpec>>,
    qr_module: u8,

    // Mirrors of device state, so redundant ops are never emitted. Initial values are
    // what `ESC @` leaves behind, which the emitter always sends first.
    dev_justify: Align,
    dev_mag: (u8, u8),
    dev_attrs: Attrs,
}

impl<'a> Engine<'a> {
    fn new(profile: &'a Profile) -> Self {
        Self {
            profile,
            ops: Vec::new(),
            justify: Align::Left,
            mag: (1, 1),
            cols: None,
            qr_module: qr::DEFAULT_MODULE,
            dev_justify: Align::Left,
            dev_mag: (1, 1),
            dev_attrs: Attrs::default(),
        }
    }

    fn run(&mut self, ast: &[Node]) -> Result<(), Error> {
        for node in ast {
            self.node(node)?;
        }
        Ok(())
    }

    fn node(&mut self, node: &Node) -> Result<(), Error> {
        let line = node.line;
        match &node.block {
            // The version marker is a compatibility assertion, not an instruction.
            Block::Version(_) => {}

            Block::Justify(a) => self.justify = *a,
            Block::Size { w, h } => self.mag = (*w, *h),
            Block::Cols(specs) => self.cols = Some(specs.clone()),
            Block::ColsEnd => self.cols = None,

            Block::Feed(n) => self.ops.push(Op::Feed(*n)),
            Block::Cut => self.ops.push(Op::Cut(CutKind::Partial)),
            Block::Raw(bytes) => self.ops.push(Op::Raw(bytes.clone())),

            Block::QrModule(n) => self.qr_module = *n,
            Block::Qr(data) => self.qr(line, data)?,

            Block::Blank => {
                self.set_attrs(Attrs::default());
                self.text("\n");
            }

            Block::Rule => {
                let width = self.grid_columns();
                self.set_justify(Align::Left);
                self.set_size();
                self.set_attrs(Attrs::default());
                let rule: String = "-".repeat(width);
                self.text(&rule);
                self.text("\n");
            }

            Block::Line(cells) => match self.cols.clone() {
                Some(specs) => self.row(line, &specs, cells)?,
                None => self.plain(cells),
            },
        }
        Ok(())
    }

    // --- plain lines ---------------------------------------------------------------

    /// A line outside column mode. Justification is left to the device: `ESC a` centers
    /// the line buffer, which is both cheaper and more accurate than computing a center
    /// position here, and it is the one case where the device's own arithmetic is
    /// guaranteed to agree with ours.
    fn plain(&mut self, cells: &[Cell]) {
        let width = self.grid_columns();
        let glyphs: Vec<Glyph> = cells.iter().flat_map(flatten).collect();

        self.set_justify(self.justify);
        self.set_size();

        if glyphs.is_empty() {
            self.set_attrs(Attrs::default());
            self.text("\n");
            return;
        }

        for line in wrap(&glyphs, width) {
            self.glyphs(&line);
            self.text("\n");
        }
    }

    // --- qr --------------------------------------------------------------------------

    /// A QR symbol. Block-level: it occupies its own line and is never a cell.
    ///
    /// Placement is delegated to the device exactly as [`plain`](Self::plain) does it —
    /// `GS ( k` prints through the line buffer, so `ESC a` centers the symbol and no dot
    /// arithmetic is needed here. That was confirmed on hardware rather than assumed.
    ///
    /// Magnification is deliberately not applied: `GS !` does not scale a QR, whose size
    /// comes from its own module-size parameter. `{size 2x2}` around a `{qr}` is a no-op
    /// on the symbol, which is why the grid is not consulted.
    fn qr(&mut self, line: usize, data: &str) -> Result<(), Error> {
        // The payload is measured in UTF-8 bytes because that is what goes into the
        // symbol — QR byte mode is opaque bytes, not code-page characters.
        let bytes = data.len();
        let available = self.profile.width_dots;

        let needed = qr::footprint_dots(bytes, self.qr_module).ok_or(Error::QrTooLong {
            line,
            bytes,
            max_bytes: qr::max_bytes(),
        })?;
        if needed > available {
            return Err(Error::QrTooWide {
                line,
                module: self.qr_module,
                needed_dots: needed,
                available_dots: available,
            });
        }

        self.set_justify(self.justify);
        // Emphasis cannot reach a QR, but leaving the device mid-attribute across a
        // block-level element is the habit that produces receipts printed entirely bold.
        self.set_attrs(Attrs::default());
        self.ops.push(Op::Qr {
            data: data.to_string(),
            module: self.qr_module,
        });
        Ok(())
    }

    // --- column rows ---------------------------------------------------------------

    fn row(&mut self, line: usize, specs: &[ColSpec], cells: &[Cell]) -> Result<(), Error> {
        // The parser guarantees the counts match; if that ever stops being true, this is
        // a logic error rather than a template error.
        debug_assert_eq!(specs.len(), cells.len());

        let available = self.grid_columns();
        let requested: usize = specs.iter().map(|s| s.width_chars as usize).sum();
        if requested > available {
            return Err(Error::ColumnsTooWide {
                line,
                requested: saturating_u16(requested),
                available: saturating_u16(available),
            });
        }

        // Wrap every cell first, because the row is as tall as its tallest cell and that
        // is not known until all of them have been broken.
        let mut wrapped: Vec<Vec<Vec<Glyph>>> = Vec::with_capacity(specs.len());
        for (idx, (spec, cell)) in specs.iter().zip(cells).enumerate() {
            let width = spec.width_chars as usize;
            let glyphs = flatten(cell);

            if spec.align == Align::Right {
                // Right-aligned columns hold prices and totals. Wrapping one silently
                // turns 65.000 into two lines that read as different numbers, so this
                // is an error rather than a truncation.
                if width_of(&glyphs) > width {
                    return Err(Error::RightColumnOverflow {
                        line,
                        column: idx + 1,
                        width: spec.width_chars,
                        content: text_of(&glyphs),
                    });
                }
                wrapped.push(if glyphs.is_empty() {
                    Vec::new()
                } else {
                    vec![glyphs]
                });
            } else {
                wrapped.push(wrap(&glyphs, width));
            }
        }

        let height = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let cell_dots = self.cell_dots();

        // Absolute positioning only means anything against the left margin, so the
        // device must not also be centering the line buffer.
        self.set_justify(Align::Left);
        self.set_size();

        for row in 0..height {
            let mut start = 0usize;
            for (idx, spec) in specs.iter().enumerate() {
                let width = spec.width_chars as usize;
                if let Some(glyphs) = wrapped[idx].get(row).filter(|g| !g.is_empty()) {
                    let offset = match spec.align {
                        Align::Left => 0,
                        Align::Right => width.saturating_sub(width_of(glyphs)),
                        Align::Center => width.saturating_sub(width_of(glyphs)) / 2,
                    };
                    // Emitted for every cell including the first at position zero.
                    // Explicit positioning is the whole point of §5.1, and four bytes is
                    // cheaper than reasoning about what the line buffer was left holding.
                    self.ops
                        .push(Op::AbsPos(saturating_u16((start + offset) * cell_dots)));
                    self.glyphs(glyphs);
                }
                start += width;
            }
            self.text("\n");
        }

        Ok(())
    }

    // --- op emission ----------------------------------------------------------------

    fn glyphs(&mut self, glyphs: &[Glyph]) {
        let mut buf = String::new();
        let mut attrs = glyphs.first().map(|(_, a)| *a).unwrap_or_default();

        for (ch, a) in glyphs {
            if *a != attrs {
                self.set_attrs(attrs);
                self.text(&std::mem::take(&mut buf));
                attrs = *a;
            }
            buf.push(*ch);
        }
        self.set_attrs(attrs);
        self.text(&buf);
    }

    /// Append text, merging into the previous op when it is also text.
    ///
    /// Merging keeps the IR — and therefore every golden fixture — free of runs of
    /// single-character `Text` ops, which makes a real diff visible in a failure.
    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        match self.ops.last_mut() {
            Some(Op::Text(prev)) => prev.push_str(s),
            _ => self.ops.push(Op::Text(s.to_string())),
        }
    }

    fn set_justify(&mut self, a: Align) {
        if self.dev_justify != a {
            self.ops.push(Op::Justify(a));
            self.dev_justify = a;
        }
    }

    fn set_size(&mut self) {
        if self.dev_mag != self.mag {
            let (w, h) = self.mag;
            self.ops.push(Op::Size { w, h });
            self.dev_mag = self.mag;
        }
    }

    fn set_attrs(&mut self, a: Attrs) {
        if self.dev_attrs.bold != a.bold {
            self.ops.push(Op::Emphasis(a.bold));
            self.dev_attrs.bold = a.bold;
        }
        if self.dev_attrs.underline != a.underline {
            self.ops.push(Op::Underline(a.underline));
            self.dev_attrs.underline = a.underline;
        }
    }

    // --- geometry -------------------------------------------------------------------

    /// Dots per character cell at the current magnification.
    fn cell_dots(&self) -> usize {
        usize::from(self.profile.font.char_width_dots()) * usize::from(self.mag.0.max(1))
    }

    /// Characters per line at the current magnification. Never a constant.
    fn grid_columns(&self) -> usize {
        usize::from(self.profile.columns_at(self.mag.0))
    }

    // --- framing --------------------------------------------------------------------

    /// Close the document: attributes off, then a feed and a cut if the template did not
    /// end with one. A receipt that leaves emphasis on makes the *next* receipt bold,
    /// possibly on someone else's order.
    fn finish(mut self) -> Vec<Op> {
        // Order matters. Turning attributes off appends ops, so testing for a trailing
        // cut afterwards would find the reset instead and cut the receipt twice — which
        // is exactly what a template ending in `**TOTAL**` used to do.
        if matches!(self.ops.last(), Some(Op::Cut(_))) {
            let cut = self.ops.pop().expect("just matched");
            self.set_attrs(Attrs::default());
            self.ops.push(cut);
        } else {
            self.set_attrs(Attrs::default());
            self.ops.push(Op::Feed(FINAL_FEED));
            self.ops.push(Op::Cut(CutKind::Partial));
        }
        self.ops
    }
}

// --- text measurement and wrapping ---------------------------------------------------

fn flatten(cell: &Cell) -> Vec<Glyph> {
    cell.spans
        .iter()
        .flat_map(|Span { text, bold, underline }| {
            let attrs = Attrs {
                bold: *bold,
                underline: *underline,
            };
            text.chars().map(move |c| (c, attrs))
        })
        .collect()
}

/// Display width in character cells.
fn width_of(glyphs: &[Glyph]) -> usize {
    glyphs.iter().map(|(c, _)| char_width(*c)).sum()
}

fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

fn text_of(glyphs: &[Glyph]) -> String {
    glyphs.iter().map(|(c, _)| *c).collect()
}

/// Break glyphs into lines of at most `width` cells.
///
/// Breaks at the last space that fits; a word with no break opportunity is split hard
/// rather than allowed to run past the margin. Interior spacing inside a line is
/// preserved — collapsing runs of spaces would quietly wreck hand-aligned columns
/// inside a single cell.
fn wrap(glyphs: &[Glyph], width: usize) -> Vec<Vec<Glyph>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut rest = glyphs;

    while !rest.is_empty() {
        if width_of(rest) <= width {
            lines.push(trim_end(rest).to_vec());
            return lines;
        }

        // Longest prefix that fits.
        let mut fit = 0;
        let mut used = 0;
        for (i, (c, _)) in rest.iter().enumerate() {
            let cw = char_width(*c);
            if used + cw > width {
                break;
            }
            used += cw;
            fit = i + 1;
        }
        // A single glyph wider than the whole column: take it anyway, or loop forever.
        let fit = fit.max(1);

        let (take, skip) = match rest[..fit].iter().rposition(|(c, _)| *c == ' ') {
            // Drop the space the break landed on.
            Some(at) if at > 0 => (at, at + 1),
            // No break opportunity in range, so split the word.
            _ => (fit, fit),
        };

        lines.push(trim_end(&rest[..take]).to_vec());
        rest = &rest[skip..];
        // Leading spaces on a continuation line would read as accidental indentation.
        while matches!(rest.first(), Some((' ', _))) {
            rest = &rest[1..];
        }
    }

    lines
}

fn trim_end(glyphs: &[Glyph]) -> &[Glyph] {
    let mut end = glyphs.len();
    while end > 0 && glyphs[end - 1].0 == ' ' {
        end -= 1;
    }
    &glyphs[..end]
}

fn saturating_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn ops(src: &str) -> Vec<Op> {
        layout(&parse(src).unwrap(), &Profile::epson_80mm()).unwrap()
    }

    fn err(src: &str) -> Error {
        layout(&parse(src).unwrap(), &Profile::epson_80mm()).unwrap_err()
    }

    /// Ops with the closing feed/cut stripped, which every document otherwise carries.
    fn body(src: &str) -> Vec<Op> {
        let mut v = ops(src);
        if matches!(v.last(), Some(Op::Cut(_))) {
            v.pop();
            if matches!(v.last(), Some(Op::Feed(_))) {
                v.pop();
            }
        }
        v
    }

    fn text(s: &str) -> Op {
        Op::Text(s.into())
    }

    #[test]
    fn plain_text_leaves_justification_to_the_device() {
        assert_eq!(
            body("{center}\nTOKO MAJU"),
            vec![Op::Justify(Align::Center), text("TOKO MAJU\n")]
        );
    }

    #[test]
    fn redundant_state_changes_are_not_emitted() {
        // Left is what ESC @ leaves behind, and 1x1 likewise.
        assert_eq!(body("{left}{size 1x1}\nA"), vec![text("A\n")]);
        // The same justification twice is one op.
        assert_eq!(
            body("{center}\nA\n{center}\nB"),
            vec![Op::Justify(Align::Center), text("A\nB\n")]
        );
    }

    #[test]
    fn columns_resolve_to_absolute_dot_positions() {
        // Font A is 12 dots. Column starts at chars 0, 20, 30 -> 0, 240, 360 dots.
        // Right-aligned cells sit at (start + width - content) * 12.
        let out = body("{cols 20,10:r,12:r}\nNasi Goreng | 2 x 25.000 | 50.000");
        assert_eq!(
            out,
            vec![
                Op::AbsPos(0),
                text("Nasi Goreng"),
                // 20 + 10 - 10 = 20 chars
                Op::AbsPos(240),
                text("2 x 25.000"),
                // 30 + 12 - 6 = 36 chars
                Op::AbsPos(432),
                text("50.000\n"),
            ]
        );
    }

    #[test]
    fn magnification_scales_both_the_grid_and_the_positions() {
        // At 2x a cell is 24 dots, so a 10-char column starts at 240, not 120.
        let out = body("{size 2x2}{cols 10,10:r}\nA | B");
        assert_eq!(
            out,
            vec![
                Op::Size { w: 2, h: 2 },
                Op::AbsPos(0),
                text("A"),
                // Column 1 starts at char 10, right-aligned content of width 1 -> char 19.
                Op::AbsPos(19 * 24),
                text("B\n"),
            ]
        );
    }

    #[test]
    fn magnification_shrinks_the_rule_to_the_current_grid() {
        assert_eq!(body("---"), vec![text(&format!("{}\n", "-".repeat(48)))]);
        assert_eq!(
            body("{size 2x2}\n---"),
            vec![
                Op::Size { w: 2, h: 2 },
                text(&format!("{}\n", "-".repeat(24)))
            ]
        );
    }

    #[test]
    fn columns_wider_than_the_magnified_paper_are_rejected() {
        // 42 columns fits at 1x and cannot at 2x, where only 24 are available.
        assert!(layout(&parse("{cols 20,10:r,12:r}\na|b|c").unwrap(), &Profile::epson_80mm()).is_ok());

        assert_eq!(
            err("{size 2x2}{cols 20,10:r,12:r}\na|b|c"),
            Error::ColumnsTooWide {
                line: 2,
                requested: 42,
                available: 24,
            }
        );
    }

    #[test]
    fn left_columns_wrap_with_a_hanging_indent() {
        // Continuation lines return to the column's own start position, not to zero.
        let out = body("{cols 10,10:r}\nNasi Goreng Spesial | 50.000");
        assert_eq!(
            out,
            vec![
                Op::AbsPos(0),
                text("Nasi"),
                Op::AbsPos((10 + 10 - 6) * 12),
                text("50.000\n"),
                Op::AbsPos(0),
                text("Goreng\n"),
                Op::AbsPos(0),
                text("Spesial\n"),
            ]
        );
    }

    #[test]
    fn right_columns_refuse_to_wrap() {
        let e = err("{cols 20,6:r}\nItem | 1.250.000");
        assert_eq!(
            e,
            Error::RightColumnOverflow {
                line: 2,
                column: 2,
                width: 6,
                content: "1.250.000".into(),
            }
        );
    }

    #[test]
    fn centered_columns_wrap_rather_than_erroring() {
        let out = body("{cols 6:c,10}\nabc def | x");
        assert_eq!(
            out,
            vec![
                // "abc" is 3 wide in a 6-wide column -> offset 1.
                Op::AbsPos(12),
                text("abc"),
                Op::AbsPos(6 * 12),
                text("x\n"),
                Op::AbsPos(12),
                text("def\n"),
            ]
        );
    }

    #[test]
    fn wide_characters_are_measured_in_cells_not_chars() {
        // Two CJK glyphs occupy four cells, so a right-aligned 10-wide column starts at
        // char 6, not char 8.
        let out = body("{cols 10:r}\n日本");
        assert_eq!(out, vec![Op::AbsPos(6 * 12), text("日本\n")]);
    }

    #[test]
    fn a_word_longer_than_its_column_is_split_rather_than_overrun() {
        let out = body("{cols 4}\nabcdefghij");
        assert_eq!(
            out,
            vec![
                Op::AbsPos(0),
                text("abcd\n"),
                Op::AbsPos(0),
                text("efgh\n"),
                Op::AbsPos(0),
                text("ij\n"),
            ]
        );
    }

    #[test]
    fn inline_attributes_toggle_around_their_run() {
        assert_eq!(
            body("**TOTAL** now"),
            vec![
                Op::Emphasis(true),
                text("TOTAL"),
                Op::Emphasis(false),
                text(" now\n"),
            ]
        );
    }

    #[test]
    fn documents_end_self_contained() {
        // Emphasis left on at the end of the template is turned back off before the cut.
        let out = ops("**loud");
        assert_eq!(
            out,
            vec![
                Op::Emphasis(true),
                text("loud\n"),
                Op::Emphasis(false),
                Op::Feed(FINAL_FEED),
                Op::Cut(CutKind::Partial),
            ]
        );
    }

    #[test]
    fn a_bold_ending_does_not_cause_a_second_cut() {
        // Regression: turning emphasis off used to be appended before the trailing-cut
        // check, so the check saw the reset and cut the paper a second time.
        assert_eq!(
            ops("**TOTAL**\n{cut}"),
            vec![
                Op::Emphasis(true),
                text("TOTAL\n"),
                Op::Emphasis(false),
                Op::Cut(CutKind::Partial),
            ]
        );
    }

    #[test]
    fn an_explicit_cut_is_not_doubled() {
        let out = ops("A\n{feed 2}\n{cut}");
        assert_eq!(
            out,
            vec![
                text("A\n"),
                Op::Feed(2),
                Op::Cut(CutKind::Partial),
            ]
        );
    }

    #[test]
    fn raw_passes_through_layout_untouched() {
        assert_eq!(body("{raw 1D564100}"), vec![Op::Raw(vec![0x1D, 0x56, 0x41, 0x00])]);
    }

    #[test]
    fn the_same_template_renders_at_two_paper_widths() {
        let src = "{cols 12,8:r}\nItem | 1.000";
        let ast = parse(src).unwrap();

        let wide = layout(&ast, &Profile::epson_80mm()).unwrap();
        let narrow = layout(
            &ast,
            &Profile {
                width_dots: 384,
                ..Profile::epson_80mm()
            },
        )
        .unwrap();

        // The layout is identical because the spec fits both; what changes is the grid
        // it was validated against. Positions are driven by the spec, not the paper.
        assert_eq!(wide, narrow);

        // A spec that only fits the wider paper is rejected on the narrower one.
        let ast = parse("{cols 30,10:r}\nItem | 1.000").unwrap();
        assert!(layout(&ast, &Profile::epson_80mm()).is_ok());
        assert!(matches!(
            layout(
                &ast,
                &Profile {
                    width_dots: 384,
                    ..Profile::epson_80mm()
                }
            ),
            Err(Error::ColumnsTooWide { .. })
        ));
    }

    // --- qr ---------------------------------------------------------------------------

    #[test]
    fn qr_uses_the_default_module_and_current_justification() {
        assert_eq!(
            body("{center}\n{qr https://terrakernel.com}"),
            vec![
                Op::Justify(Align::Center),
                Op::Qr {
                    data: "https://terrakernel.com".into(),
                    module: qr::DEFAULT_MODULE,
                }
            ]
        );
    }

    #[test]
    fn qrmod_is_sticky_across_symbols() {
        let ops = body("{qrmod 4}\n{qr ONE}\n{qr TWO}");
        assert_eq!(
            ops,
            vec![
                Op::Qr {
                    data: "ONE".into(),
                    module: 4
                },
                Op::Qr {
                    data: "TWO".into(),
                    module: 4
                },
            ]
        );
    }

    #[test]
    fn a_qr_too_wide_for_the_paper_is_rejected() {
        // 210 bytes is version 10 — 57 modules plus 8 of quiet zone, which needs 585 dots
        // at module 9 against 576 available.
        let payload = "A".repeat(210);
        let err = err(&format!("{{qrmod 9}}\n{{qr {payload}}}"));
        assert!(
            matches!(
                err,
                Error::QrTooWide {
                    line: 2,
                    module: 9,
                    needed_dots: 585,
                    available_dots: 576,
                }
            ),
            "{err}"
        );

        // The same payload one module size down fits, so the check is not just refusing
        // everything large.
        assert!(ops(&format!("{{qrmod 8}}\n{{qr {payload}}}"))
            .iter()
            .any(|op| matches!(op, Op::Qr { .. })));
    }

    #[test]
    fn a_payload_past_version_40_is_rejected() {
        let err = err(&format!("{{qr {}}}", "A".repeat(2400)));
        assert!(
            matches!(
                err,
                Error::QrTooLong {
                    line: 1,
                    bytes: 2400,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn qr_is_measured_in_utf8_bytes_not_characters() {
        // A QR carries opaque bytes, so a 3-byte character costs three of the budget.
        // Measuring in `chars()` here would under-count and let an overflowing symbol
        // through — the one number in this file that is deliberately not a display width.
        let ast = parse("{qrmod 16}\n{qr 日本語}").unwrap();
        let ops = layout(&ast, &Profile::epson_80mm()).unwrap();
        let Some(Op::Qr { data, .. }) = ops.iter().find(|o| matches!(o, Op::Qr { .. })) else {
            panic!("expected a QR op");
        };
        assert_eq!(data.len(), 9);
        assert_eq!(data.chars().count(), 3);
    }

    #[test]
    fn qr_size_is_independent_of_magnification() {
        // `GS !` does not scale a symbol, so `{size 2x2}` must not change the op at all.
        let plain = body("{qr HELLO}");
        let doubled: Vec<Op> = body("{size 2x2}\n{qr HELLO}")
            .into_iter()
            .filter(|op| !matches!(op, Op::Size { .. }))
            .collect();
        assert_eq!(plain, doubled);
    }
}
