# CLAUDE.md

Guidance for Claude Code working in this repository.

`INSTRUCTIONS.md` is the authoritative design document. Read it before writing code.
This file covers what that document leaves open, plus findings from the §9 research gates.

---

## What this is

A Rust library: `(template: &str, profile: &Profile) -> Result<Vec<u8>, Error>`.

Template string in, ESC/POS bytes out. Layout lives in a string that can sit in a database
row, so changing a receipt footer is a data edit rather than a redeploy. The layout engine
is the product; the parser is the cheap part.

## Repository state

Scaffolded. Builds clean, `cargo test` and `cargo clippy` pass.

```
mdpos/                  # core lib — no I/O, no async; deps: unicode-width (+ optional serde)
  src/
    lib.rs              # render() / preview() / to_ops() — the public contract
    parse.rs            # template -> Block AST            done, unit-tested
    layout.rs           # Block AST + Profile -> Vec<Op>   done, unit-tested — the product
    ir.rs               # Op, Align, CutKind               done
    profile.rs          # Profile, Dialect, Font, CodePage done
    error.rs            # Error, with source line numbers  done
    emit/escpos.rs      # Vec<Op> -> Vec<u8>               done, unit-tested
    emit/preview.rs     # Vec<Op> -> String (monospace)    done, unit-tested
  tests/golden.rs       # fixture harness
mdpos-cli/              # thin binary, owns all file I/O
tests/golden/           # 4 fixtures, structured as if publishable
```

The v0.1 pipeline is complete end to end: a template file renders to bytes and to a preview.
What remains before §12 can be claimed is a print on real hardware, and the CP437 high range
in `emit::escpos::encode_text` (non-ASCII is currently rejected, which is also what blocks the
unicode golden fixture).

Keep the workspace split. The moment `mdpos` gains a dependency that touches the filesystem,
§1.1 is already lost. `serde` is an optional feature used only to deserialize fixture profiles;
it is not part of the rendering contract.

## Commands

```
cargo test --workspace          # unit + golden fixtures
cargo clippy --workspace --all-targets
cargo run -p mdpos-cli -- template.txt > out.bin
cargo run -p mdpos-cli -- --preview template.txt
UPDATE_GOLDEN=1 cargo test --test golden    # regenerate fixtures, then read the diff
```

Golden fixtures are regenerated deliberately, never automatically. If a change rewrites a
`.bin`, that is a v1 compatibility break until proven otherwise — inspect the diff by hand.

## Decisions made during scaffolding

Not in `INSTRUCTIONS.md`; revisit freely, but know they were choices:

- **`Op::Text` carries its own `\n`.** The IR in §3.1 has no line-break variant, so an embedded
  newline is the break and emitters map it to `LF`. Layout owns where breaks fall.
- **Document framing lives in the emitter, not the IR.** `ESC @` plus font and code-page
  selection are prepended by `emit::escpos`. Init has no meaning in the preview backend, and
  keeping it out of the op stream means a template cannot forge it. The trailing feed and cut
  are layout's responsibility, since those *are* device-independent.
- **Non-ASCII is an error, not a `?`.** `encode_text` covers ASCII; the CP437 high range
  (0x80..=0xFF) still needs its table. A silently substituted `?` on a printed receipt becomes a
  support call that never traces back to the encoder.
- **Partial cut falls back to full** when `supports_partial_cut` is false, rather than emitting
  a command the mechanism ignores.
- **`Profile::columns_at(mag)`** exists so layout has no excuse to recompute the magnified grid
  by hand at each call site.

Made while writing the parser:

- **Content lines and cells are trimmed at both ends.** This is the format's whole differentiator
  from ReceiptLine made concrete: whitespace is never load-bearing, alignment is stated with
  `:r`. `\` escapes the following character, so `\ Total` still gets a literal leading space.
- **One escape rule, not a table.** `\` makes the next character literal — covers `\|`, `\*`,
  `\_`, `\{`, `\\`.
- **`**` and `__` toggle, and reset at every cell boundary.** An unclosed `**` bolds the rest of
  its cell and stops. Attribute leakage is exactly the bug class §5.5 is about.
- **`Block::Blank`** distinguishes an empty source line from a row of empty cells. Without it,
  layout has to guess.
- **`{v N}` must be the first directive**, and a version above `MAX_VERSION` is an error. The
  string carries the compatibility promise, so it is worth being strict about early.
- **`{cols}` state lives in the parser**, because it decides whether `|` is a separator or
  literal text — syntax, not layout. Cell-count mismatches are caught here, with a line number.
- **`---` needs three dashes**, so a lone `-` in a cell stays literal.
- **A leading UTF-8 BOM is stripped.** Templates pasted from Windows editors carry one and it
  would otherwise make `{v 1}` unrecognizable — a confusing failure for a format edited by hand.

Made while writing the layout engine:

- **Column widths are in *current* characters.** `{cols 20,...}` under `{size 2x2}` means 20
  double-width characters — 40 base cells, 480 dots — and is validated against
  `columns_at(2)` = 24, so that spec is correctly rejected. The alternative reading (widths
  fixed in base cells) would make `{cols}` and `{size}` silently independent.
- **Plain lines delegate justification to the device** (`ESC a`), while column rows force
  `Justify(Left)` and position with `AbsPos`. Absolute positioning is measured from the left
  margin, so the device must not also be centering the line buffer — the two cannot both be
  in play on one line.
- **`AbsPos` is emitted for every cell, including the first at dot 0.** Four bytes is cheaper
  than reasoning about what the line buffer was left holding.
- **Text ops coalesce.** `text()` merges into the previous op when it is also `Text`, so
  fixtures don't fill with single-character ops and a real diff stays visible.
- **Device-state mirrors** (`dev_justify`, `dev_mag`, `dev_attrs`) start at what `ESC @` leaves
  behind, so `{left}` or `{size 1x1}` at the top of a template emits nothing.
- **`FINAL_FEED = 4`** lines before an auto-appended cut. The cut command already feeds; this
  is so the last printed line isn't flush against the tear edge.
- **The preview works in base cells throughout**, matching `AbsPos`. Magnified text is measured
  at its printed width and then drawn narrow, so a 2x line starts where it really starts and
  ends early. Position is what layout bugs corrupt, so position is what the preview gets right.

---

## Constraints that override ordinary judgment

These come from `INSTRUCTIONS.md` §1 and were settled before the repo existed. They are not
defaults to be improved on.

**No transport in the core.** No tokio, serialport, USB, sockets, or file handles in `mdpos`.
The largest target hardware (Sunmi, iMin, Telpo on Android) exposes only `sendRAWData(byte[])`
through a vendor AIDL service — producing bytes is the only thing that works there. Queueing,
chunking, retry, `DLE EOT` status polling, and job atomicity all belong to the caller.

**Always go through the `Op` IR.** Never compile template text to bytes directly, even when it
would obviously work for the case in front of you. The IR is what makes the preview backend,
future dialects, and a vendor-SDK backend possible at all.

**`Row` and `Cell` must not appear in the IR.** Columns resolve during layout into `Text` +
`AbsPos` sequences. The IR is post-layout and width-independent by construction. If a cell
concept is leaking into `ir.rs`, the layout pass is incomplete.

**Never hardcode 48 columns.** Derive: `width_dots / font.char_width_dots`. Same template must
render 58mm and 80mm.

**`{v 1}` is honored forever.** The string is the public API and carries the semver, not the
crate. Any syntax change that would alter how an existing v1 template renders is a bug, not a
release note.

**Every document is self-contained.** Begins with `ESC @`, ends with feed + cut, assumes nothing
about prior device state. The printer is a stateful interpreter — a forgotten bold flag makes
every subsequent receipt bold until power cycle. A lost or duplicated document must not be able
to corrupt the next one.

## Where the bugs will be

Concentrated in `layout.rs`. In rough order of how often they bite:

- **Magnification mutates the grid mid-document.** `{size 2x2}` halves effective columns 48 → 24.
  Track *current* character width, never a document constant. This is the most common source of
  subtly wrong output.
- **Right-alignment is dot arithmetic, not space padding.** Use `ESC $ nL nH`
  (`nL + nH*256` dots from left margin). Space padding breaks the instant magnification or font
  changes mid-line.
- **Width is `unicode-width`.** Never `str::len()`, never `chars().count()`. Indonesian retail is
  mostly ASCII right up until a customer name isn't.
- **Overflow policy is per-column.** Wrap `:l`/`:c` with hanging indent aligned to the column's
  start position; **never wrap `:r`** — a wrapped total is worse than a rejected template. v0.1
  errors on right-column overflow.

## Scope discipline

v0.1 is deliberately small: one hardcoded 80mm Epson profile, the syntax subset in §4, two
backends, a CLI, golden tests.

Out — not "later," but *not decided*: FFI/C ABI, WASM, Android, QR/barcode/bitmaps, profile
registry or TOML loading, data interpolation of any kind, web services, Star/ZPL/TSPL dialects,
publishing the conformance corpus.

Do not add any of these opportunistically, and do not add scaffolding "so it's ready later."
None of it can be evaluated until the layout engine exists and is known to be good.

`{raw HEX}` is the exception that is genuinely in scope. It is load-bearing, not a hack: clone
printers (Xprinter, Rongta, EPPOS, Gainscha — the actual installed base) deviate on cut variants,
`ESC $` handling, and native QR, and that is unfixable in principle. It costs one `Op` variant
and it means a vendor quirk never blocks a release.

---

## §9 research findings (July 2026)

Both gates in `INSTRUCTIONS.md` §9 have been checked. Summary and recommendation below; the
underlying claim is that the layout-engine gap is real, and it is.

### ReceiptLine — evaluated, close, recommend not adopting wholesale

[receiptline/receiptline](https://github.com/receiptline/receiptline), Apache-2.0, from OFSC
(OpenFoodserviceSystemConsortium, Japan). ~766 stars, active, JS reference implementation with a
"ReceiptLine Designer" tool offering preview, hex dump against a virtual printer, and test
printing. It is approximately the thing `INSTRUCTIONS.md` describes and has a fair claim to being
the de facto standard in this niche.

Its capability set overlaps almost completely:

| Concern | ReceiptLine | mdpos §4 |
|---|---|---|
| Columns | `\|`-delimited, `{width: 10,20,18}` or `auto`/`*` | `{cols 20,10:r,12:r}` |
| Alignment | inferred from whitespace around pipes | explicit `:l` `:r` `:c` |
| Magnification | `^` … `^^^^^^^` (2x–6x) | `{size WxH}` (1–8) |
| Emphasis / underline / invert | `"` / `_` / `` ` `` | `**` / `__` / — |
| Rule / cut | `-` / `=` | `---` / `{cut}` |
| Wrap policy | `{text: wrap\|nowrap}` | per-column, `:r` errors |
| Raw escape hatch | `{command: …}` | `{raw HEX}` |
| Paper width | separate config (`cpl`), document is width-agnostic | separate `Profile` |
| Outputs | svg, text, escpos + epson/sii/citizen/fit/impact, star variants, png | bytes + monospace preview |
| Images / barcodes / QR | `{image:}` `{code:}` with options | out of v0.1 scope |

Three concrete conflicts with settled constraints:

1. **No version marker in the document.** ReceiptLine versions at the SDK/tool level, not in the
   string. This is exactly the failure mode §1.3 exists to prevent — if the string is the public
   API, the string carries the semver. Adding `{v 1}` to ReceiptLine means it is no longer
   ReceiptLine, which forfeits the entire compatibility argument for adopting it.

2. **Alignment is encoded in whitespace, and the whole premise is templates living in database
   rows.** `column|` versus `| column` versus `|column ` are three different alignments
   distinguished by leading and trailing spaces. Trailing whitespace does not survive a textarea,
   a form submit, a YAML round-trip, a copy-paste, or most ORM-adjacent trimming. mdpos's stated
   deployment model puts these strings in exactly those places. This is the strongest argument
   against adoption and it is not in `INSTRUCTIONS.md` — explicit `:r` markers are robust where
   significant trailing spaces are not.

3. **Per-line formatting versus sticky state.** ReceiptLine does not carry formatting across
   lines; §4 specifies justification, size, and `{cols}` as sticky. Worth noting the per-line
   model is arguably the *safer* design given §5.5, and is worth stealing even while rejecting
   the rest of the grammar. Sticky state is a convenience that costs a class of bug.

**No Rust implementation exists.** crates.io has nothing; the closest neighbor is `escpresso`, an
ESC/POS *emulator* GUI. So "ReceiptLine for Rust" is genuinely unclaimed territory if that path
is ever taken.

**Recommendation:** build the syntax in §4 as specified, and mine ReceiptLine's property model
(`{width:}`, `{text: wrap|nowrap}`, `{border:}`) and its clone-printer edge cases rather than its
surface grammar. Because of §1.2, a ReceiptLine-compatible front-end is later a parser swap
feeding the same IR — it stays cheap, and it does not need deciding now. This is a judgment call
on incomplete information and worth an explicit sign-off before the parser is written.

### Gap confirmation — holds, no library has grown a template-string layout engine

| Library | Status | Layout facilities |
|---|---|---|
| `escpos` (fabienbellanger) v0.19.0, May 2026 | builder-only | Grew a `ui` module — `UIComponent` trait, lines, tables. Still Rust method calls, not a string. |
| `escpos-rs` (Malanche) | builder-only | Its `Instruction` "template" is variable substitution into a builder-constructed structure, not a layout language. |
| `python-escpos` 3.2.dev | builder-only | `software_columns(text_list, widths, align)` and `block_text()` wrapping — real layout logic, but reached only by method call. |
| `escpos-php` (mike42), PHP 8.2+ | builder-only | Shares `escpos-printer-db` with python-escpos. |
| `node-thermal-printer` | builder-only | `tableCustom([{text, align, width}])`, `leftRight()`. Proportional widths (0.5, 0.25). |

Adjacent prior art worth knowing: `node-escpos-templates` (roydejong) is genuinely string-driven
at runtime, with opcodes, `loop`/`endloop`, and `if`/`endif` — and has **no column, table, or
width support whatsoever**. It is the clearest evidence for the thesis: someone built the
template-string half and stopped exactly where the difficulty starts.

Two things did shift since `INSTRUCTIONS.md` was written and are worth absorbing rather than
ignoring: `escpos`'s `ui` module and python-escpos's `software_columns` both indicate the demand
is real and that column layout is drifting into these libraries bottom-up. Neither crosses into
template-string territory, so the differentiator stands — but the empty space is narrowing from
that direction, not from ReceiptLine's.

---

## Testing

Golden files from commit one, both backends snapshotting the same fixture:

```
tests/golden/001-basic-columns/{input.tmpl, profile.ron, expected.bin, expected.txt}
```

Output must be byte-deterministic. Structure this directory as if it will be published — it is
the seed of both the conformance corpus and the customer-facing compatibility test
(*"print this; if it matches, you're supported"*), even though publishing is out of v0.1 scope.

Priority cases: basic columns, double-width grid mutation, unicode wrapping, right-align
overflow rejection. Those are the four places §5 says the engine will be wrong.

## Reference

Use the Epson ESC/POS Command Reference, not blog posts. v0.1 command set is tabulated in
`INSTRUCTIONS.md` §6.

Note that "standard ESC/POS" is not a real standard — it is Epson's proprietary command set,
copied to varying degrees, with no spec body and no certification. Keep compatibility claims
falsifiable.

## Success criterion

A hand-edited template file, rendered by CLI, printed on a real 80mm printer, producing correctly
right-aligned prices, a double-width header, wrapped long product names, and a clean cut — with
zero recompilation between layout edits.

Nothing deferred in §2 is worth discussing before that works and the layout code isn't horrible.
