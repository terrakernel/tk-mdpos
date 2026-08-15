//! Template text into a block AST. The cheap part.
//!
//! The parser resolves *syntax* only. It must not consult the [`Profile`](crate::Profile)
//! or compute a single width: `{cols 20,10:r}` becomes a spec of two columns in
//! characters, and what that means in dots is entirely the layout pass's problem.
//!
//! # Whitespace is never load-bearing
//!
//! Content lines and cells are trimmed at both ends. This is deliberate and it is the
//! main reason this format exists rather than adopting ReceiptLine, where alignment is
//! encoded in the spaces around a `|`. These templates are meant to live in database
//! rows, config fields, and text areas — none of which reliably preserve trailing
//! whitespace. Alignment is stated with `:r`, never implied by padding.
//!
//! If a literal leading space is genuinely wanted, `\` escapes the character after it,
//! so `\ Total` survives the trim.
//!
//! # Escaping
//!
//! One rule: `\` makes the next character literal. That covers `\|` inside a column row,
//! `\*` and `\_` against inline markup, `\{` against a directive at line start, and `\\`
//! for a backslash itself.

use crate::ir::Align;
use crate::qr;
use crate::Error;

/// A parsed block with the source line it came from, for error reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// 1-based source line.
    pub line: usize,
    pub block: Block,
}

/// One structural element of a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `{v N}` — format version. Optional in v0.1, assumed 1.
    Version(u32),
    /// `{left}` `{center}` `{right}` — sticky until changed.
    Justify(Align),
    /// `{size WxH}` — magnification, 1..=8 each. Sticky.
    Size { w: u8, h: u8 },
    /// `{cols A,B:r,C:c}` — widths in *characters*. Sticky until `{/cols}` or the next
    /// `{cols}`.
    Cols(Vec<ColSpec>),
    /// `{/cols}` — leave column mode.
    ColsEnd,
    /// `---` — full-width separator.
    Rule,
    /// `{feed N}`.
    Feed(u8),
    /// `{cut}`.
    Cut,
    /// `{raw 1D564200}` — already hex-decoded.
    Raw(Vec<u8>),
    /// `{qr DATA}` — a QR symbol. The payload is stored verbatim, with `\` escapes
    /// already resolved; the printer encodes it, so the parser does not care what it says.
    Qr(String),
    /// `{qrmod N}` — QR module size in dots, 1..=16. Sticky, like `{size}`.
    QrModule(u8),
    /// An empty source line. Distinct from a row of empty cells.
    Blank,
    /// A content line. One cell when no `{cols}` is active, otherwise split on
    /// unescaped `|`, with exactly as many cells as the active spec has columns.
    Line(Vec<Cell>),
}

/// One column of an active `{cols}` spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColSpec {
    /// Width in characters at magnification 1x.
    pub width_chars: u16,
    /// `:l` (default), `:r`, or `:c`.
    pub align: Align,
}

/// The contents of one cell: a sequence of attributed text runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub spans: Vec<Span>,
}

/// A run of text sharing one set of inline attributes.
///
/// Inline markup is a deliberately tiny markdown subset — `**bold**` and `__underline__`
/// and nothing else. Attributes are flattened into runs here rather than kept as a tree,
/// because layout measures widths run by run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub underline: bool,
}

/// The format version assumed when a template omits `{v N}`.
pub const DEFAULT_VERSION: u32 = 1;

/// The highest format version this build implements.
///
/// Raising this is a promise: every template at or below it must render identically
/// forever after (`INSTRUCTIONS.md` §1.3).
pub const MAX_VERSION: u32 = 1;

/// Parse template text into a block AST.
pub fn parse(template: &str) -> Result<Vec<Node>, Error> {
    let mut p = Parser::default();
    p.run(template)?;
    Ok(p.nodes)
}

#[derive(Default)]
struct Parser {
    nodes: Vec<Node>,
    /// The active `{cols}` spec, if any. The parser tracks this because it decides
    /// whether `|` is a separator or a literal — it is a syntax concern, not layout.
    cols: Option<Vec<ColSpec>>,
    /// Whether anything at all has been emitted, so `{v}` can be pinned to the top.
    emitted: bool,
}

impl Parser {
    fn run(&mut self, template: &str) -> Result<(), Error> {
        for (i, raw) in template.lines().enumerate() {
            let line = i + 1;
            // Strip a UTF-8 BOM; templates arriving from Windows editors carry one and
            // it would otherwise make the leading `{v 1}` unrecognizable.
            let raw = if i == 0 {
                raw.trim_start_matches('\u{feff}')
            } else {
                raw
            };
            self.line(line, raw)?;
        }
        Ok(())
    }

    fn line(&mut self, line: usize, raw: &str) -> Result<(), Error> {
        let mut rest = raw.trim();

        // A line may be prefixed by any number of directives: `{center}{size 2x2}Text`.
        let mut had_directive = false;
        while rest.starts_with('{') {
            let end = directive_end(rest).ok_or_else(|| Error::BadDirective {
                line,
                name: rest.trim_start_matches('{').chars().take(16).collect(),
                detail: "unterminated directive — missing `}`".into(),
            })?;
            let block = self.directive(line, rest[1..end].trim())?;
            self.push(line, block);
            had_directive = true;
            rest = rest[end + 1..].trim_start();
        }

        if rest.is_empty() {
            // A directive-only line contributes no content; a genuinely empty source
            // line is a blank line on paper.
            if !had_directive {
                self.push(line, Block::Blank);
            }
            return Ok(());
        }

        if is_rule(rest) {
            self.push(line, Block::Rule);
            return Ok(());
        }

        let block = match &self.cols {
            Some(specs) => {
                let raw_cells = split_cells(rest);
                if raw_cells.len() != specs.len() {
                    return Err(Error::ColumnCountMismatch {
                        line,
                        expected: specs.len(),
                        found: raw_cells.len(),
                    });
                }
                Block::Line(raw_cells.iter().map(|c| parse_cell(c)).collect())
            }
            None => Block::Line(vec![parse_cell(rest)]),
        };
        self.push(line, block);
        Ok(())
    }

    fn push(&mut self, line: usize, block: Block) {
        // Column state is syntax, so it is tracked here rather than reconstructed by
        // layout from the node stream.
        match &block {
            Block::Cols(specs) => self.cols = Some(specs.clone()),
            Block::ColsEnd => self.cols = None,
            _ => {}
        }
        self.emitted = true;
        self.nodes.push(Node { line, block });
    }

    fn directive(&mut self, line: usize, inner: &str) -> Result<Block, Error> {
        let (name, args) = match inner.find(char::is_whitespace) {
            Some(i) => (&inner[..i], inner[i..].trim()),
            None => (inner, ""),
        };

        let bad = |detail: &str| Error::BadDirective {
            line,
            name: name.to_string(),
            detail: detail.to_string(),
        };

        match name {
            "v" => {
                if self.emitted {
                    return Err(bad("must be the first directive in the template"));
                }
                let requested: u32 = args
                    .parse()
                    .map_err(|_| bad("expected a version number, as in `{v 1}`"))?;
                if requested > MAX_VERSION {
                    return Err(Error::UnsupportedVersion { line, requested });
                }
                Ok(Block::Version(requested))
            }

            "left" => Ok(Block::Justify(Align::Left)),
            "center" => Ok(Block::Justify(Align::Center)),
            "right" => Ok(Block::Justify(Align::Right)),

            "size" => {
                let (w, h) = args
                    .split_once(['x', 'X'])
                    .ok_or_else(|| bad("expected WxH, as in `{size 2x2}`"))?;
                let w: u8 = w
                    .trim()
                    .parse()
                    .map_err(|_| bad("width must be a number 1..=8"))?;
                let h: u8 = h
                    .trim()
                    .parse()
                    .map_err(|_| bad("height must be a number 1..=8"))?;
                if !(1..=8).contains(&w) || !(1..=8).contains(&h) {
                    return Err(Error::SizeOutOfRange { line, w, h });
                }
                Ok(Block::Size { w, h })
            }

            "cols" => {
                if args.is_empty() {
                    return Err(bad("expected at least one column, as in `{cols 20,28:r}`"));
                }
                let specs = args
                    .split(',')
                    .map(|part| col_spec(part.trim(), &bad))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Block::Cols(specs))
            }
            "/cols" => Ok(Block::ColsEnd),

            "feed" => {
                let n: u8 = args
                    .parse()
                    .map_err(|_| bad("expected a line count, as in `{feed 4}`"))?;
                Ok(Block::Feed(n))
            }

            "cut" => {
                if args.is_empty() {
                    Ok(Block::Cut)
                } else {
                    Err(bad("takes no arguments"))
                }
            }

            "raw" => Ok(Block::Raw(hex(args, line)?)),

            "qr" => {
                if args.is_empty() {
                    return Err(bad("expected data, as in `{qr https://example.com}`"));
                }
                Ok(Block::Qr(unescape(args)))
            }

            "qrmod" => {
                let n: u8 = args
                    .parse()
                    .map_err(|_| bad("expected a module size, as in `{qrmod 6}`"))?;
                if !(qr::MIN_MODULE..=qr::MAX_MODULE).contains(&n) {
                    return Err(bad(&format!(
                        "module size must be {}..={}, got {n}",
                        qr::MIN_MODULE,
                        qr::MAX_MODULE
                    )));
                }
                Ok(Block::QrModule(n))
            }

            _ => Err(Error::UnknownDirective {
                line,
                name: name.to_string(),
            }),
        }
    }
}

fn col_spec(part: &str, bad: &dyn Fn(&str) -> Error) -> Result<ColSpec, Error> {
    let (width, align) = match part.split_once(':') {
        Some((w, a)) => {
            let align = match a.trim() {
                "l" => Align::Left,
                "r" => Align::Right,
                "c" => Align::Center,
                other => {
                    return Err(bad(&format!(
                        "unknown column alignment `:{other}` — expected `:l`, `:r`, or `:c`"
                    )))
                }
            };
            (w.trim(), align)
        }
        None => (part, Align::Left),
    };

    let width_chars: u16 = width
        .parse()
        .map_err(|_| bad(&format!("column width `{width}` is not a number")))?;
    if width_chars == 0 {
        return Err(bad("column width must be at least 1"));
    }

    Ok(ColSpec {
        width_chars,
        align,
    })
}

/// Index of the `}` that closes a directive, honoring the `\` escape.
///
/// Only `{qr}` has a payload arbitrary enough to contain a brace, but the scan is shared
/// rather than special-cased on the directive name, which would mean sniffing the name
/// before knowing where the directive ends. This cannot change how any *valid* v1 template
/// renders: no other directive accepts a backslash in its arguments, so the only templates
/// affected are ones that were already errors.
fn directive_end(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '}' => return Some(i),
            _ => {}
        }
    }
    None
}

/// Resolve `\` escapes in a directive payload, matching the rule used inside cells.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(escaped) => out.push(escaped),
                None => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    out
}

/// Decode a `{raw}` payload. Whitespace between bytes is allowed, so `{raw 1D 56 42 00}`
/// and `{raw 1D564200}` are the same thing — the spaced form is far easier to check
/// against a vendor's command table.
fn hex(args: &str, line: usize) -> Result<Vec<u8>, Error> {
    let digits: String = args.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() {
        return Err(Error::BadHex {
            line,
            detail: "no bytes given".into(),
        });
    }
    if digits.len() % 2 != 0 {
        return Err(Error::BadHex {
            line,
            detail: format!("odd number of hex digits ({})", digits.len()),
        });
    }
    let bytes = digits.as_bytes();
    (0..bytes.len() / 2)
        .map(|i| {
            let pair = &digits[i * 2..i * 2 + 2];
            u8::from_str_radix(pair, 16).map_err(|_| Error::BadHex {
                line,
                detail: format!("`{pair}` is not a hex byte"),
            })
        })
        .collect()
}

/// A separator line: three or more dashes and nothing else.
///
/// Three rather than one, so that a lone `-` in a cell stays literal.
fn is_rule(s: &str) -> bool {
    s.len() >= 3 && s.bytes().all(|b| b == b'-')
}

/// Split a row on unescaped `|`, leaving escapes in place for [`parse_cell`].
fn split_cells(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '|' => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Parse one cell's text into attributed runs.
///
/// `**` and `__` toggle. Attributes never escape the cell they were opened in: an
/// unclosed `**` bolds the rest of that cell and stops there. The printer is a stateful
/// interpreter and a leaked attribute is the kind of bug that survives until someone
/// power-cycles the hardware, so the parser refuses to carry state across cells at all.
fn parse_cell(s: &str) -> Cell {
    let s = s.trim();
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut underline = false;

    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(escaped) => buf.push(escaped),
                // A trailing backslash is a literal backslash.
                None => buf.push('\\'),
            },
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                flush_span(&mut spans, &mut buf, bold, underline);
                bold = !bold;
            }
            '_' if chars.peek() == Some(&'_') => {
                chars.next();
                flush_span(&mut spans, &mut buf, bold, underline);
                underline = !underline;
            }
            c => buf.push(c),
        }
    }
    flush_span(&mut spans, &mut buf, bold, underline);

    Cell { spans }
}

fn flush_span(spans: &mut Vec<Span>, buf: &mut String, bold: bool, underline: bool) {
    if !buf.is_empty() {
        spans.push(Span {
            text: std::mem::take(buf),
            bold,
            underline,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(src: &str) -> Vec<Block> {
        parse(src).unwrap().into_iter().map(|n| n.block).collect()
    }

    fn plain(text: &str) -> Block {
        Block::Line(vec![Cell {
            spans: vec![Span {
                text: text.into(),
                bold: false,
                underline: false,
            }],
        }])
    }

    #[test]
    fn directives_may_prefix_content() {
        assert_eq!(
            blocks("{size 2x2}TOKO MAJU"),
            vec![Block::Size { w: 2, h: 2 }, plain("TOKO MAJU")]
        );
        assert_eq!(
            blocks("{center}{size 2x2}HI"),
            vec![
                Block::Justify(Align::Center),
                Block::Size { w: 2, h: 2 },
                plain("HI")
            ]
        );
    }

    #[test]
    fn directive_only_line_yields_no_content() {
        assert_eq!(blocks("{feed 4}"), vec![Block::Feed(4)]);
        assert_eq!(blocks("{cut}"), vec![Block::Cut]);
    }

    #[test]
    fn blank_lines_are_distinct_from_empty_rows() {
        assert_eq!(blocks("A\n\nB"), vec![plain("A"), Block::Blank, plain("B")]);
    }

    #[test]
    fn rule_needs_three_dashes() {
        assert_eq!(blocks("---"), vec![Block::Rule]);
        assert_eq!(blocks("-----"), vec![Block::Rule]);
        assert_eq!(blocks("--"), vec![plain("--")]);
    }

    #[test]
    fn columns_parse_widths_and_alignment() {
        assert_eq!(
            blocks("{cols 20,10:r,12:c}"),
            vec![Block::Cols(vec![
                ColSpec {
                    width_chars: 20,
                    align: Align::Left
                },
                ColSpec {
                    width_chars: 10,
                    align: Align::Right
                },
                ColSpec {
                    width_chars: 12,
                    align: Align::Center
                },
            ])]
        );
    }

    #[test]
    fn pipes_split_only_while_cols_is_active() {
        // Outside column mode a pipe is ordinary text.
        assert_eq!(blocks("a | b"), vec![plain("a | b")]);

        let out = blocks("{cols 10,10:r}\na | b");
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], Block::Line(vec![cell("a"), cell("b")]));
    }

    #[test]
    fn cols_end_restores_literal_pipes() {
        let out = blocks("{cols 10,10}\na|b\n{/cols}\nc|d");
        assert_eq!(out[3], plain("c|d"));
    }

    fn cell(text: &str) -> Cell {
        Cell {
            spans: vec![Span {
                text: text.into(),
                bold: false,
                underline: false,
            }],
        }
    }

    #[test]
    fn cells_are_trimmed_so_padding_is_never_load_bearing() {
        let out = blocks("{cols 20,10:r,12:r}\nNasi Goreng    | 2 x 25.000 | 50.000");
        assert_eq!(
            out[1],
            Block::Line(vec![cell("Nasi Goreng"), cell("2 x 25.000"), cell("50.000")])
        );
    }

    #[test]
    fn wrong_cell_count_is_an_error_with_a_line_number() {
        let err = parse("{cols 10,10}\na|b|c").unwrap_err();
        assert_eq!(
            err,
            Error::ColumnCountMismatch {
                line: 2,
                expected: 2,
                found: 3
            }
        );
    }

    #[test]
    fn escaped_pipe_stays_in_the_cell() {
        let out = blocks("{cols 20,10}\na \\| b | c");
        assert_eq!(out[1], Block::Line(vec![cell("a | b"), cell("c")]));
    }

    #[test]
    fn inline_markup_produces_attributed_runs() {
        let out = blocks("**TOTAL** now");
        assert_eq!(
            out[0],
            Block::Line(vec![Cell {
                spans: vec![
                    Span {
                        text: "TOTAL".into(),
                        bold: true,
                        underline: false
                    },
                    Span {
                        text: " now".into(),
                        bold: false,
                        underline: false
                    },
                ]
            }])
        );
    }

    #[test]
    fn underline_and_bold_nest() {
        let out = blocks("**__both__**");
        assert_eq!(
            out[0],
            Block::Line(vec![Cell {
                spans: vec![Span {
                    text: "both".into(),
                    bold: true,
                    underline: true
                }]
            }])
        );
    }

    #[test]
    fn attributes_do_not_leak_across_cells() {
        let out = blocks("{cols 10,10}\n**loud | quiet");
        assert_eq!(
            out[1],
            Block::Line(vec![
                Cell {
                    spans: vec![Span {
                        text: "loud".into(),
                        bold: true,
                        underline: false
                    }]
                },
                cell("quiet"),
            ])
        );
    }

    #[test]
    fn single_asterisk_is_literal() {
        assert_eq!(blocks("2 * 3"), vec![plain("2 * 3")]);
        assert_eq!(blocks("\\*\\*not bold\\*\\*"), vec![plain("**not bold**")]);
    }

    #[test]
    fn backslash_space_survives_the_trim() {
        assert_eq!(blocks("\\ indented"), vec![plain(" indented")]);
        // Each escape covers exactly one character.
        assert_eq!(blocks("\\ \\ deeper"), vec![plain("  deeper")]);
    }

    #[test]
    fn raw_accepts_spaced_and_packed_hex() {
        assert_eq!(
            blocks("{raw 1D564200}"),
            vec![Block::Raw(vec![0x1D, 0x56, 0x42, 0x00])]
        );
        assert_eq!(
            blocks("{raw 1D 56 42 00}"),
            vec![Block::Raw(vec![0x1D, 0x56, 0x42, 0x00])]
        );
    }

    #[test]
    fn version_must_lead_and_must_be_supported() {
        assert_eq!(blocks("{v 1}"), vec![Block::Version(1)]);

        let err = parse("{v 2}").unwrap_err();
        assert_eq!(
            err,
            Error::UnsupportedVersion {
                line: 1,
                requested: 2
            }
        );

        let err = parse("hello\n{v 1}").unwrap_err();
        assert!(matches!(err, Error::BadDirective { line: 2, .. }));
    }

    #[test]
    fn bom_does_not_hide_the_version_marker() {
        assert_eq!(blocks("\u{feff}{v 1}"), vec![Block::Version(1)]);
    }

    #[test]
    fn size_is_range_checked() {
        assert_eq!(
            parse("{size 9x1}").unwrap_err(),
            Error::SizeOutOfRange { line: 1, w: 9, h: 1 }
        );
        assert!(matches!(
            parse("{size 2}").unwrap_err(),
            Error::BadDirective { .. }
        ));
    }

    #[test]
    fn errors_carry_the_source_line() {
        let err = parse("{center}\nok\n{nope}").unwrap_err();
        assert_eq!(
            err,
            Error::UnknownDirective {
                line: 3,
                name: "nope".into()
            }
        );

        let err = parse("{center\n").unwrap_err();
        assert!(matches!(err, Error::BadDirective { line: 1, .. }));

        assert!(matches!(
            parse("{raw 1D5}").unwrap_err(),
            Error::BadHex { line: 1, .. }
        ));
        assert!(matches!(
            parse("{raw ZZ}").unwrap_err(),
            Error::BadHex { line: 1, .. }
        ));
    }

    #[test]
    fn the_readme_template_parses() {
        let src = "\
{v 1}
{center}
{size 2x2}TOKO MAJU
{size 1x1}Jl. Sudirman 42
---
{left}
{cols 20,10:r,12:r}
Nasi Goreng    | 2 x 25.000 | 50.000
Es Teh Manis   | 3 x  5.000 | 15.000
---
{cols 22,20:r}
**TOTAL**      | **65.000**
{feed 4}
{cut}
";
        let nodes = parse(src).unwrap();
        assert!(matches!(nodes.first().map(|n| &n.block), Some(Block::Version(1))));
        assert!(matches!(nodes.last().map(|n| &n.block), Some(Block::Cut)));
        // Two item rows, plus the total row.
        let rows = nodes
            .iter()
            .filter(|n| matches!(n.block, Block::Line(_)))
            .count();
        assert_eq!(rows, 5);
    }

    #[test]
    fn qr_payload_keeps_its_internal_spaces() {
        // Real QRIS payloads carry the merchant name inline, so a payload containing a
        // space is the normal case rather than an edge one.
        assert_eq!(
            blocks("{qr 5910TOKO MAJU6013Jakarta Pusat}"),
            vec![Block::Qr("5910TOKO MAJU6013Jakarta Pusat".into())]
        );
    }

    #[test]
    fn qr_payload_may_contain_an_escaped_brace() {
        // The directive scanner has to respect `\` or the payload would end early.
        assert_eq!(
            blocks(r#"{qr {\"amount\":5000\}}"#),
            vec![Block::Qr(r#"{"amount":5000}"#.into())]
        );
    }

    #[test]
    fn qr_without_data_is_rejected() {
        let err = parse("{qr}").unwrap_err();
        assert!(matches!(err, Error::BadDirective { line: 1, .. }), "{err}");
    }

    #[test]
    fn qrmod_is_range_checked() {
        assert_eq!(blocks("{qrmod 6}"), vec![Block::QrModule(6)]);
        assert_eq!(blocks("{qrmod 16}"), vec![Block::QrModule(16)]);

        for bad in ["{qrmod 0}", "{qrmod 17}", "{qrmod x}"] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn a_directive_may_still_be_unterminated() {
        // The escape-aware scan must not turn a missing `}` into a silent success.
        let err = parse("{qr no closing brace").unwrap_err();
        assert!(matches!(err, Error::BadDirective { line: 1, .. }), "{err}");
    }
}
