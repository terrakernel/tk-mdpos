//! HTML backend, for showing a person what the paper will look like.
//!
//! The monospace backend in [`preview`](super::preview) is a developer's diff tool: it is
//! honest about the grid and throws away everything else. This one exists for the other
//! audience — someone deciding whether a receipt *looks right* before a printer is
//! involved — so it draws the three things monospace discards: emphasis, underline, and
//! magnification at its real size.
//!
//! # Resemblance, not pixel fidelity
//!
//! We do not have the printer's ROM bitmap font and never will, so the glyphs a browser
//! draws are approximate. That is an acceptable standard here because **the preview is not
//! the safety net for fit**: layout already wraps `:l`/`:c` overflow and already rejects
//! `:r` overflow and an oversized QR. Nothing can silently run off the paper edge in a
//! document that renders at all, which leaves this backend responsible only for the
//! question a person is actually asking — what will this look like.
//!
//! # Character cells, not dots
//!
//! Everything horizontal is expressed in `ch` units and positions are resolved to whole
//! character cells, exactly as [`preview`](super::preview) resolves [`Op::AbsPos`]. `ch`
//! *is* the advance width of a monospace font, so the grid lands correctly whatever font
//! the browser happens to have, and the two preview backends cannot disagree about where
//! column 20 is. Expressing positions in dot-derived pixels instead would make the layout
//! depend on the host's font metrics being what we guessed.
//!
//! Vertical is the one place with no printer grid to honor, so line height is simply fixed
//! at `2ch` — Font A's cell is 12x24 dots, so a 1:2 ratio is what a receipt looks like.
//!
//! # What is deliberately not drawn
//!
//! [`Op::Qr`] renders as a correctly-sized empty square and [`Op::Raw`] as a labelled
//! band. Both could be given plausible-looking artwork; neither should be. A drawn QR that
//! is not the symbol the printer will generate invites someone to point a phone at it, and
//! an invented image misreports what `{raw}` contains. Showing the footprint honestly
//! answers the real question — how much paper it takes and where it sits — and the payload
//! is carried on a `data-` attribute so a host with its own QR library can draw the true
//! symbol over it.

use crate::ir::{Align, CutKind, Op};
use crate::profile::Profile;
use crate::Error;

/// Scoped stylesheet, shipped inside the fragment.
///
/// Everything is scoped under `.mdpos` so the fragment can be dropped into a host page
/// without colliding with it, and so it still renders standalone when written to a file or
/// handed to a WebView. A preview a stray host stylesheet can silently shift is worth less
/// than no preview.
///
/// The paper is deliberately light in both host themes. A receipt is white paper, and
/// resembling the output is the entire job.
const STYLE: &str = "\
.mdpos{--mdpos-lh:2ch;display:inline-block;padding:16px;background:#f4f4f5;\
font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,\"DejaVu Sans Mono\",monospace;\
font-size:14px;line-height:var(--mdpos-lh);color:#111}\
.mdpos-paper{width:calc(var(--mdpos-cols) * 1ch);padding:12px 10px;background:#fff;\
box-shadow:0 1px 4px rgba(0,0,0,.25)}\
.mdpos-line{position:relative;height:var(--mdpos-lh)}\
.mdpos-run{position:absolute;top:0;left:0;white-space:pre;transform-origin:left top}\
.mdpos-b{font-weight:700}\
.mdpos-u{text-decoration:underline}\
.mdpos-qr{aspect-ratio:1;box-sizing:border-box;border:1px dashed #9a9a9a;\
display:flex;align-items:center;justify-content:center;color:#6b6b6b;font-size:.8em}\
.mdpos-raw{box-sizing:border-box;border:1px dashed #9a9a9a;color:#6b6b6b;font-size:.8em;\
text-align:center;padding:4px 0}\
.mdpos-cut{border-top:1px dashed #9a9a9a;margin:6px 0;text-align:center;color:#9a9a9a;\
font-size:.7em}\
";

/// Render an op stream as a self-contained HTML fragment.
pub fn emit(ops: &[Op], profile: &Profile) -> Result<String, Error> {
    let mut e = Emitter::new(profile);

    for op in ops {
        match op {
            Op::Text(s) => {
                // `split` yields one more segment than there are newlines, so the tail
                // after a trailing `\n` is an empty segment that pushes nothing — which is
                // what leaves an unterminated final line pending for the flush below.
                for (i, seg) in s.split('\n').enumerate() {
                    if i > 0 {
                        e.commit_line();
                    }
                    e.push_text(seg);
                }
            }

            // Resolved to a whole character cell, the same units the monospace backend
            // uses, so the two can never disagree about where a column starts.
            Op::AbsPos(dots) => e.next_pos = Some(usize::from(*dots / e.cell_dots)),

            Op::Justify(a) => e.justify = *a,
            Op::Size { w, h } => {
                e.mag_w = (*w).max(1);
                e.mag_h = (*h).max(1);
            }
            Op::Emphasis(on) => e.bold = *on,
            Op::Underline(on) => e.underline = *on,

            Op::Feed(n) => {
                e.flush_pending();
                for _ in 0..*n {
                    e.blank_line();
                }
            }

            Op::Cut(kind) => {
                e.flush_pending();
                e.cut(*kind);
            }

            Op::Raw(bytes) => {
                e.flush_pending();
                e.raw(bytes.len());
            }

            Op::Qr { data, module } => {
                e.flush_pending();
                e.qr(data, *module);
            }
        }
    }

    e.flush_pending();
    Ok(e.finish())
}

/// One stretch of text sharing a single set of attributes.
///
/// `pos` is set only where layout positioned the text explicitly — i.e. cells of a column
/// row. A run without one continues from wherever the previous run ended.
struct Run {
    text: String,
    bold: bool,
    underline: bool,
    mag_w: u8,
    mag_h: u8,
    pos: Option<usize>,
}

struct Emitter {
    out: String,
    cols: usize,
    cell_dots: u16,
    justify: Align,
    bold: bool,
    underline: bool,
    mag_w: u8,
    mag_h: u8,
    next_pos: Option<usize>,
    runs: Vec<Run>,
}

impl Emitter {
    fn new(profile: &Profile) -> Self {
        let cols = profile.columns() as usize;
        let mut out = String::new();
        out.push_str(&format!(
            "<div class=\"mdpos\" style=\"--mdpos-cols:{cols}\">\n<style>{STYLE}</style>\n<div class=\"mdpos-paper\">\n"
        ));

        Self {
            out,
            cols,
            cell_dots: profile.font.char_width_dots(),
            justify: Align::Left,
            bold: false,
            underline: false,
            mag_w: 1,
            mag_h: 1,
            next_pos: None,
            runs: Vec::new(),
        }
    }

    fn finish(mut self) -> String {
        self.out.push_str("</div>\n</div>\n");
        self.out
    }

    fn push_text(&mut self, text: &str) {
        // An empty segment still consumes a pending position — the cell was positioned,
        // it just turned out to have nothing in it.
        let pos = self.next_pos.take();
        if text.is_empty() {
            return;
        }
        self.runs.push(Run {
            text: text.to_string(),
            bold: self.bold,
            underline: self.underline,
            mag_w: self.mag_w,
            mag_h: self.mag_h,
            pos,
        });
    }

    /// Commit the pending line. Always emits one, even when empty — a blank source line is
    /// a blank line on paper.
    fn commit_line(&mut self) {
        if self.runs.is_empty() {
            self.blank_line();
            return;
        }

        let runs = std::mem::take(&mut self.runs);

        // A line carrying explicit positions is a column row: layout forced it to
        // `Justify(Left)` and placed every cell itself, so re-justifying here would move
        // text the engine already decided the position of.
        let positioned = runs.iter().any(|r| r.pos.is_some());
        let mut cursor = if positioned {
            0
        } else {
            let printed: usize = runs.iter().map(printed_cells).sum();
            match self.justify {
                Align::Left => 0,
                Align::Center => self.cols.saturating_sub(printed) / 2,
                Align::Right => self.cols.saturating_sub(printed),
            }
        };

        // A magnified character is physically taller, so the line it sits on is taller.
        // The monospace backend can ignore this; paper cannot.
        let height = runs.iter().map(|r| r.mag_h).max().unwrap_or(1).max(1);
        if height > 1 {
            self.out.push_str(&format!(
                "<div class=\"mdpos-line\" style=\"height:calc({height} * var(--mdpos-lh))\">"
            ));
        } else {
            self.out.push_str("<div class=\"mdpos-line\">");
        }

        for run in &runs {
            let x = run.pos.unwrap_or(cursor);
            cursor = x + printed_cells(run);

            let mut class = String::from("mdpos-run");
            if run.bold {
                class.push_str(" mdpos-b");
            }
            if run.underline {
                class.push_str(" mdpos-u");
            }

            let mut style = format!("left:{x}ch");
            if run.mag_w > 1 || run.mag_h > 1 {
                style.push_str(&format!(";transform:scale({},{})", run.mag_w, run.mag_h));
            }

            self.out.push_str(&format!(
                "<span class=\"{class}\" style=\"{style}\">{}</span>",
                escape(&run.text)
            ));
        }

        self.out.push_str("</div>\n");
    }

    /// Commit a line only if one is pending, for ops that imply a line boundary without
    /// being one. Committing unconditionally would bake a phantom blank line into every
    /// fixture.
    fn flush_pending(&mut self) {
        if !self.runs.is_empty() {
            self.commit_line();
        }
    }

    fn blank_line(&mut self) {
        self.out.push_str("<div class=\"mdpos-line\"></div>\n");
    }

    fn cut(&mut self, kind: CutKind) {
        let label = match kind {
            CutKind::Partial => "partial cut",
            CutKind::Full => "full cut",
        };
        self.out
            .push_str(&format!("<div class=\"mdpos-cut\">{label}</div>\n"));
    }

    /// A `{raw}` band. Its dimensions are genuinely unknown — the bytes are opaque by
    /// design and this is also the image mechanism — so the honest rendering is a
    /// full-width placeholder that says how much is there and admits it is not drawn.
    fn raw(&mut self, len: usize) {
        self.out.push_str(&format!(
            "<div class=\"mdpos-raw\">raw &middot; {len} bytes &middot; not rendered</div>\n"
        ));
    }

    /// A QR symbol at its true printed footprint, quiet zone included.
    ///
    /// Square in both axes, unlike the monospace backend's three-line sketch: here there
    /// is a real vertical scale to be honest about.
    fn qr(&mut self, data: &str, module: u8) {
        // Layout already proved the symbol fits, so the footprint is known good; the
        // fallback only keeps this backend total.
        let dots = crate::qr::footprint_dots(data.len(), module).unwrap_or(0);
        let width = usize::from(dots).div_ceil(usize::from(self.cell_dots));

        let left = match self.justify {
            Align::Left => 0,
            Align::Center => self.cols.saturating_sub(width) / 2,
            Align::Right => self.cols.saturating_sub(width),
        };

        let payload = escape(data);
        self.out.push_str(&format!(
            "<div class=\"mdpos-qr\" style=\"margin-left:{left}ch;width:{width}ch\" \
             data-mdpos-qr=\"{payload}\" title=\"{payload}\">QR</div>\n"
        ));
    }
}

/// Cells a run covers on paper. A magnified character occupies `mag_w` of them.
fn printed_cells(run: &Run) -> usize {
    width_of(&run.text) * usize::from(run.mag_w.max(1))
}

/// Display width, per `unicode-width`. Never `len()`, never `chars().count()`.
fn width_of(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Escape for both text content and double-quoted attribute values.
///
/// One function rather than two: a QR payload is a URL often enough that it reaches an
/// attribute, and the set that is safe in an attribute is safe in text as well.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Just the paper contents, so assertions are not swamped by the stylesheet.
    fn body(ops: &[Op]) -> String {
        let html = emit(ops, &Profile::epson_80mm()).unwrap();
        let start = html.find("<div class=\"mdpos-paper\">\n").unwrap()
            + "<div class=\"mdpos-paper\">\n".len();
        let end = html.rfind("</div>\n</div>\n").unwrap();
        html[start..end].to_string()
    }

    #[test]
    fn abspos_resolves_to_a_character_cell() {
        // Font A is 12 dots per cell, so 240 dots is cell 20.
        let out = body(&[
            Op::Text("Item".into()),
            Op::AbsPos(240),
            Op::Text("9.000\n".into()),
        ]);
        assert!(out.contains("style=\"left:0ch\">Item</span>"), "{out}");
        assert!(out.contains("style=\"left:20ch\">9.000</span>"), "{out}");
    }

    #[test]
    fn an_unpositioned_line_is_justified_like_the_device_would() {
        let centered = body(&[Op::Justify(Align::Center), Op::Text("HI\n".into())]);
        // 48 cells, 2 of text, so 23 either side.
        assert!(centered.contains("left:23ch"), "{centered}");

        let right = body(&[Op::Justify(Align::Right), Op::Text("END\n".into())]);
        assert!(right.contains("left:45ch"), "{right}");
    }

    #[test]
    fn centering_accounts_for_the_printed_width_of_magnified_text() {
        // At 2x, "HI" covers 4 cells on paper, so it starts at 22 rather than 23. This is
        // the same arithmetic the monospace backend does, and the two must agree.
        let out = body(&[
            Op::Justify(Align::Center),
            Op::Size { w: 2, h: 2 },
            Op::Text("HI\n".into()),
        ]);
        assert!(out.contains("left:22ch"), "{out}");
        assert!(out.contains("transform:scale(2,2)"), "{out}");
    }

    #[test]
    fn a_magnified_line_reserves_the_height_it_prints_at() {
        let tall = body(&[Op::Size { w: 2, h: 2 }, Op::Text("BIG\n".into())]);
        assert!(
            tall.contains("style=\"height:calc(2 * var(--mdpos-lh))\""),
            "{tall}"
        );

        // Double width alone is not double height, and must not reserve it.
        let wide = body(&[Op::Size { w: 2, h: 1 }, Op::Text("WIDE\n".into())]);
        assert!(!wide.contains("height:calc"), "{wide}");
        assert!(wide.contains("transform:scale(2,1)"), "{wide}");
    }

    #[test]
    fn emphasis_and_underline_survive_into_the_markup() {
        // These are exactly what the monospace backend has to throw away.
        let out = body(&[
            Op::Emphasis(true),
            Op::Text("TOTAL".into()),
            Op::Emphasis(false),
            Op::Underline(true),
            Op::Text(" x\n".into()),
            Op::Underline(false),
        ]);
        assert!(out.contains("class=\"mdpos-run mdpos-b\""), "{out}");
        assert!(out.contains("class=\"mdpos-run mdpos-u\""), "{out}");
    }

    #[test]
    fn runs_on_one_line_advance_past_each_other() {
        // Two runs, no explicit positions: the second starts where the first ended.
        let out = body(&[
            Op::Emphasis(true),
            Op::Text("AB".into()),
            Op::Emphasis(false),
            Op::Text("CD\n".into()),
        ]);
        assert!(out.contains("style=\"left:0ch\">AB</span>"), "{out}");
        assert!(out.contains("style=\"left:2ch\">CD</span>"), "{out}");
    }

    #[test]
    fn wide_characters_count_as_two_cells() {
        let out = body(&[Op::Justify(Align::Right), Op::Text("日本\n".into())]);
        assert!(out.contains("left:44ch"), "{out}");
    }

    #[test]
    fn qr_is_square_at_its_true_footprint() {
        // "HI" is 2 bytes -> version 1 -> 21 modules + 8 quiet = 29, at module 6 = 174
        // dots, which is 14.5 cells and so rounds up to 15.
        let out = body(&[Op::Qr {
            data: "HI".into(),
            module: 6,
        }]);
        assert!(out.contains("width:15ch"), "{out}");
        // Squareness comes from aspect-ratio in the stylesheet, not from a height here.
        assert!(!out.contains("height:"), "{out}");
    }

    #[test]
    fn a_centered_qr_is_offset_like_centered_text() {
        let out = body(&[
            Op::Justify(Align::Center),
            Op::Qr {
                data: "HI".into(),
                module: 6,
            },
        ]);
        // 48 cells, a 15-cell symbol, so 16 of lead — the same figure the monospace
        // backend produces for this case.
        assert!(out.contains("margin-left:16ch"), "{out}");
    }

    #[test]
    fn the_qr_payload_is_carried_for_a_host_to_draw() {
        let out = body(&[Op::Qr {
            data: "https://x.test/?a=1&b=2".into(),
            module: 6,
        }]);
        assert!(
            out.contains("data-mdpos-qr=\"https://x.test/?a=1&amp;b=2\""),
            "{out}"
        );
    }

    #[test]
    fn markup_in_a_template_cannot_escape_into_the_page() {
        let out = body(&[Op::Text("<script>alert(1)</script>\n".into())]);
        assert!(!out.contains("<script>"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
    }

    #[test]
    fn raw_reports_its_size_rather_than_inventing_a_picture() {
        let out = body(&[Op::Raw(vec![0u8; 128])]);
        assert!(out.contains("128 bytes"), "{out}");
        assert!(out.contains("not rendered"), "{out}");
    }

    #[test]
    fn feeds_and_cuts_are_visible() {
        let out = body(&[Op::Feed(2), Op::Cut(CutKind::Partial)]);
        assert_eq!(out.matches("<div class=\"mdpos-line\"></div>").count(), 2);
        assert!(out.contains("partial cut"), "{out}");

        let full = body(&[Op::Cut(CutKind::Full)]);
        assert!(full.contains("full cut"), "{full}");
    }

    #[test]
    fn the_paper_width_is_derived_from_the_profile() {
        let narrow = Profile {
            width_dots: 384,
            ..Profile::epson_80mm()
        };
        let out = emit(&[Op::Text("HI\n".into())], &narrow).unwrap();
        assert!(out.contains("--mdpos-cols:32"), "{out}");

        let wide = emit(&[Op::Text("HI\n".into())], &Profile::epson_80mm()).unwrap();
        assert!(wide.contains("--mdpos-cols:48"), "{wide}");
    }

    #[test]
    fn the_fragment_carries_its_own_scoped_styles() {
        let out = emit(&[Op::Text("HI\n".into())], &Profile::epson_80mm()).unwrap();
        assert!(out.starts_with("<div class=\"mdpos\""), "{out}");
        assert!(out.contains("<style>"), "{out}");
        // Every rule is scoped, so dropping this into a host page cannot restyle it.
        for rule in STYLE.split('}').filter(|r| r.contains('{')) {
            let selector = rule.split('{').next().unwrap();
            assert!(
                selector.split(',').all(|s| s.trim().starts_with(".mdpos")),
                "unscoped selector: {selector}"
            );
        }
    }

    #[test]
    fn output_is_deterministic() {
        let ops = [
            Op::Justify(Align::Center),
            Op::Size { w: 2, h: 2 },
            Op::Text("TOKO\n".into()),
            Op::Size { w: 1, h: 1 },
            Op::AbsPos(0),
            Op::Text("a".into()),
            Op::AbsPos(240),
            Op::Text("b\n".into()),
            Op::Cut(CutKind::Partial),
        ];
        let p = Profile::epson_80mm();
        assert_eq!(emit(&ops, &p).unwrap(), emit(&ops, &p).unwrap());
    }
}
