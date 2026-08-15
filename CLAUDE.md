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
tk-mdpos/                  # core lib — no I/O, no async; deps: unicode-width (+ optional serde)
  src/
    lib.rs                 # render() / preview() / to_ops() — the public contract
    parse.rs               # template -> Block AST            done, unit-tested
    layout.rs              # Block AST + Profile -> Vec<Op>   done, unit-tested — the product
    ir.rs                  # Op, Align, CutKind               done
    profile.rs             # Profile, Dialect, Font, CodePage done
    qr.rs                  # QR symbol geometry only          done, unit-tested
    error.rs               # Error, with source line numbers  done
    emit/escpos.rs         # Vec<Op> -> Vec<u8>               done, unit-tested
    emit/preview.rs        # Vec<Op> -> String (monospace)    done, unit-tested
    emit/html.rs           # Vec<Op> -> String (HTML)         done, unit-tested
  tests/golden.rs          # fixture harness
tk-mdpos-cli/              # thin binary, owns all file I/O; installs as `mdpos`
tk-mdpos-ffi/              # C ABI — cdylib + staticlib, both named libtk_mdpos
  include/tk_mdpos.h       # hand-written header, kept in step with src/lib.rs
  tests/smoke.c            # links the real staticlib; not part of cargo test
tests/golden/              # 6 fixtures, structured as if publishable
```

The v0.1 pipeline is complete end to end and **§12 has been claimed** — see "Hardware" below.
QR followed it. What remains outstanding is the CP437 high range in `emit::escpos::encode_text`
(non-ASCII in *text* is currently rejected, which is also what blocks the unicode golden
fixture; QR payloads are unaffected and go out as UTF-8).

Keep the workspace split. The moment `tk-mdpos` gains a dependency that touches the filesystem,
§1.1 is already lost. `serde` is an optional feature used only to deserialize fixture profiles;
it is not part of the rendering contract.

## Commands

```
cargo test --workspace          # unit + golden fixtures
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p tk-mdpos-cli -- template.txt > out.bin
cargo run -p tk-mdpos-cli -- --preview template.txt
UPDATE_GOLDEN=1 cargo test --test golden    # regenerate fixtures, then read the diff

# C ABI — the only check that catches header drift from the compiled library.
cargo build -p tk-mdpos-ffi
cc -Wall -Wextra -Itk-mdpos-ffi/include tk-mdpos-ffi/tests/smoke.c target/debug/libtk_mdpos.a \
   -o /tmp/tk-mdpos-smoke && /tmp/tk-mdpos-smoke
# add -fsanitize=address to check the allocation contract

# The ABI documents a thread-safety guarantee, so it is checked rather than asserted.
# Needs nightly + rust-src; -Zbuild-std is required or the sanitizer ABI mismatches core.
RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test -p tk-mdpos-ffi \
  -Zbuild-std --target aarch64-apple-darwin
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

Made while writing the C ABI:

- **One out-buffer carries both results and errors.** On any failure it holds a UTF-8 message
  instead of bytes. `Error` carries source line numbers and that text is for whoever edits the
  template — a bare status code would discard the useful half. It also means one free function
  and no thread-local `last_error`.
- **Buffers are always NUL-terminated at `ptr[len]`, with `len` excluding it.** ESC/POS output
  contains embedded zeros, so `%s` on output truncates — but it can never overrun. Terminating
  unconditionally costs one byte and removes a whole class of C caller mistake.
- **`cap` round-trips through `TkMdposBuf`** so `Vec::from_raw_parts` can rebuild the allocation
  exactly. The host must never call `free()`.
- **The FFI lib is named `tk_mdpos`** (artifact `libtk_mdpos.a`/`.dylib`) and deliberately omits
  `rlib`, which would be ambiguous with the core crate's own rlib and breaks rustdoc. Rust
  callers depend on `tk-mdpos` directly.
- **`panic = "unwind"` is pinned in the workspace `[profile.release]`.** Every entry point is
  wrapped in `catch_unwind`; `panic = "abort"` would remove that net silently.
- **`tk_mdpos_columns` and `tk_mdpos_format_version` are exported** so hosts never hardcode 48 and can
  record which format version a stored template was written against.
- **The header is hand-written, not cbindgen'd**, to avoid a build dependency. `tests/smoke.c` is
  therefore load-bearing: it is the only thing that catches the header drifting from the ABI.
- **Thread safety is a documented guarantee, not an accident** (added 2026-08-15). No global,
  thread-local, or shared mutable state exists anywhere in the core or the ABI, so every entry
  point is reentrant and hosts may call concurrently without locking. A buffer may also be freed
  on a thread other than the one that produced it, which is what a GC'd host's finalizer does.
  Two tests pin this and it is verified under ThreadSanitizer. **Do not introduce a `last_error`
  slot, a memo cache, or any other shared state** — the "errors ride in the out-buffer" decision
  is what makes the guarantee free, and breaking it now breaks published documentation.
- **Parameter names in the header avoid C++ keywords.** Three prototypes said `template`, which
  made the header uncompilable from C++ despite its `extern "C"` guards. They say `tmpl`. Names
  in a prototype are documentation only and carry no ABI weight.

---

## Constraints that override ordinary judgment

These come from `INSTRUCTIONS.md` §1 and were settled before the repo existed. They are not
defaults to be improved on.

**No transport in the core.** No tokio, serialport, USB, sockets, or file handles in `tk-mdpos`.
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

**The §12 gate passed on 2026-08-15** — see "Hardware" below. The deferred list was gated on
that print, so these are now open questions rather than blocked ones. QR has since been built;
the rest still need deciding on their merits, one at a time.

Out — not "later," but *not decided*: WASM, barcodes, profile registry or TOML loading, data
interpolation of any kind, web services, Star/ZPL/TSPL dialects, publishing the conformance
corpus.

**Images are settled and need no code: they go through `{raw}`.** See "Images" below. Do not
propose an `{image}` directive, a decoder, or a ditherer.

**`tk-mdpos-ffi` was built at the user's explicit direction ahead of the §12 gate.**
`INSTRUCTIONS.md` §2 still lists FFI as out of scope and has not been amended, so that document
and the repository disagree — as they now also do about QR and images. The concern raised at the
time was that an ABI is a compatibility anchor which would make the `Op` IR hard to change if a
real print showed the layout engine was wrong. **That print has since happened and the engine
was right**, so the concern is discharged and the C ABI is no longer provisional.

Do not add any of the remaining items opportunistically, and do not add scaffolding "so it's
ready later." Decide each on its own merits rather than because it is adjacent to something
that already exists.

`{raw HEX}` is load-bearing, not a hack, and it now carries more weight than it was designed
for: it is both the escape hatch for a vendor quirk that would otherwise block a release, and
the entire image mechanism. It costs one `Op` variant.

Note its justification has changed. It was originally argued from clone-printer deviation
(Xprinter, Rongta, EPPOS, Gainscha — "the actual installed base"). Clones are no longer chased,
so the argument is now simply that a caller sometimes needs to put bytes on the wire that this
crate has no opinion about. That is not a claim of clone support.

---

## Hardware

**§12 passed on 2026-08-15.** An acceptance template printed correctly on an Epson TM-T82X
(80mm, network, port 9100): right-aligned prices flush at column 48, a 2x2 centered header,
long names wrapping with a hanging indent, a 2x2 `TOTAL` row, and a clean partial cut. The
`ESC $` arithmetic was checked by hand against the hexdump before printing rather than trusted
from the preview. Nothing in the layout engine turned out to be wrong, which is what makes the
`Op` IR and the C ABI safe to treat as settled.

**Target is genuine Epson. Clone printers are explicitly not chased** (Julian, same day).
CLAUDE.md previously weighted Xprinter/Rongta/EPPOS/Gainscha as "the actual installed base";
that is no longer a design input. Emit the Epson command and stop — no probes, no fallbacks, no
per-vendor branches. This matters most for native QR, the single largest clone deviation area.
`{raw HEX}` stays, but its justification is now "vendor quirks are the caller's problem"
rather than "we support clones."

The printer answers `DLE EOT` normally, so a caller that wants status polling can have it — it
is still not this library's job.

## Images

Decided 2026-08-15, after a printed test sheet. Images reach the printer as `GS v 0` raster
bytes inside `{raw HEX}`. **There is no `{image}` directive and there should not be one.** The
audience is developers, so pre-processing is theirs: 1-bit conversion, dithering, padding width
to a multiple of 8, and building the `xL xH yL yH` header.

Three alternatives were considered and rejected. The reasons matter more than the outcome:

- **NV logo slots (`{logo N}`)** — recommended first and rejected, because a printer may be
  shared with non-POS uses. The objection is stronger than it first appears: a logo in printer
  flash is *per-device state set at install time*, which contradicts the constraint that every
  document begins with `ESC @` and assumes nothing about prior device state. Check
  recommendations against §1 before making them.
- **A caller-supplied asset map** (`render_with(template, profile, assets)`) — workable without
  breaking the existing signature, but unnecessary API surface when a caller can prepend the
  bytes itself.
- **A `Profile.max_document_bytes` limit** — rejected because `{raw}` cannot amplify (two hex
  characters become one byte), so it is not a safety feature, and the caller already holds the
  returned `Vec<u8>` and can check `len()` in code that knows its own transport. Revisit only
  if templates become end-user-editable, where the author and the operator stop being the same
  person.

**Layout applies justification to a `{raw}` block and does nothing else to it.** That was a fix
(55be1ae), not the original behavior: `{center}` directly above a `{raw}` used to emit nothing,
so an image sat flush left while the template asked for centering, and it only appeared to work
when a printed line above happened to leave `ESC a` set. Everything else in the payload is
passed through unexamined — including a wrong `xL`/`xH` header, which is the classic ESC/POS
image bug and prints as a diagonal smear that reads as a hardware fault.

## Decisions made while adding QR (v0.2)

- **The printer encodes, we only size.** `GS ( k` takes the payload and generates the symbol,
  so there is no encoder, no bitmap, and no new dependency. All `qr.rs` owns is the byte-mode
  version table, because layout needs the printed width in advance to reject an overflowing
  template.
- **Sizing is a deliberate upper bound.** Capacities are byte-mode, the most expensive of the
  four modes, and the printer picks the mode itself. A numeric payload may produce a smaller
  symbol than predicted, so the engine occasionally rejects a template that would have fit.
  That is the safe direction and it only bites within a few dots of the paper edge.
- **The quiet zone counts against the paper.** Epson prints the symbol at its bare module
  dimensions, so the 4-module margin per side is added to the footprint here. A code flush to
  the paper edge scans badly.
- **Error correction is fixed at M.** Right for payment codes, assumed by QRIS, and it keeps
  the capacity table one column instead of four. An EC knob means adding L/Q/H and threading
  the level through `Op::Qr` — the shape is ready, the knob is deliberately not shipped.
- **QR payloads bypass the code page.** `Op::Text` rejects non-ASCII until the CP437 high range
  exists, but QR byte mode carries opaque bytes that scanners read as UTF-8. So a `{qr}` may
  legally contain characters a text line may not, and the payload is measured in *bytes* — the
  one width in the engine that is deliberately not `unicode-width`.
- **`GS !` does not scale a QR.** Magnification is not consulted, so `{size 2x2}` around a
  `{qr}` is a no-op on the symbol. There is a test pinning that.
- **The directive scanner became escape-aware.** `{qr}` is the only directive whose payload can
  contain `}`. The scan is shared rather than special-cased on the name, which would mean
  sniffing the name before knowing where the directive ends. No *valid* v1 template is affected
  — no other directive accepts a backslash — and the four pre-existing golden fixtures were
  byte-identical afterwards, which is the evidence.
- **QR is block-level and never a cell.** It occupies its own line and honors `{center}` through
  `ESC a`, exactly as a plain line does. Confirmed on hardware, not assumed: `GS ( k` prints
  through the line buffer and advances the paper by itself, so no trailing `LF` is emitted.
- **The preview draws a box at the symbol's true width**, three lines tall regardless of real
  height. Height is not something layout can get wrong — nothing shares a line with a QR —
  and width is, so width is to scale.

## Decisions made while adding the HTML preview backend

Built 2026-08-15 at Julian's direction, on the strength of the §1.2 argument that the IR
exists to make a third backend possible. It required no change to `parse.rs`, `layout.rs`,
or `ir.rs` — purely additive, no new dependency.

- **Resemblance is the standard, not pixel fidelity.** Julian's framing: *"we don't have to
  pixel perfect as this one only a preview."* The justification is stronger than convenience
  — **the preview is not the safety net for fit.** Layout already wraps `:l`/`:c` overflow
  and rejects `:r` overflow and an oversized QR, so nothing can silently run off the paper
  edge in a document that renders at all. That leaves the backend responsible only for the
  question a person is actually asking. Do not re-litigate this by citing §8; that entry
  rejects HTML as *input*, which is a different proposal.
- **Character cells and `ch` units, not dots and pixels.** `ch` *is* a monospace font's
  advance width, so the grid lands correctly whatever font the browser has. Positioning in
  dot-derived pixels was considered and dropped: it makes layout depend on the host's font
  metrics matching a guess, and drift accumulates across a line. This also means the two
  preview backends resolve `AbsPos` identically and cannot disagree about where a column is.
- **Vertical is fixed at `2ch` per line**, because there is no printer grid to honor there.
  Font A's cell is 12x24 dots, so 1:2 is what a receipt looks like. This is the one thing
  the monospace backend can ignore entirely and paper cannot — `Op::Size.h` is real here.
- **Nothing is drawn that we are guessing at.** A QR is a correctly-sized empty square and
  `{raw}` a labelled band. A plausible-looking QR invites someone to point a phone at it and
  it will not scan; an invented image misreports what `{raw}` contains. The payload goes on
  a `data-mdpos-qr` attribute so a host with its own encoder can draw the true symbol.
- **A fragment with a scoped `<style>`, not a document and not inline styles.** The consumer
  is a WebView or an embedded page, which wants both: renders standalone, cannot be clobbered
  by host CSS. Inline styles were the first recommendation and were dropped as much larger
  output for the same collision-immunity.
- **`preview()` is not superseded.** Different audience: it is the developer's diff tool and
  the faster loop while editing, and it is what the golden fixtures assert the grid against.
- **`expected.html` pins positions, not appearance.** Whether it *looks* right is only
  answerable by opening it next to printed paper. The fixture catches a cell moving or the
  two previews drifting apart, and claims nothing more.

**The output was confirmed good by Julian on 2026-08-15**, rendered in a browser from
`mdpos --html`. So the backend is settled, and the condition that watch mode was held behind
has been met.

**Watch mode (`--html --watch`) is deferred, not rejected, and is now decidable.** It would
be the clearest demonstration of the whole thesis — edit the template, the receipt updates,
no compile — and costs one file-watcher dependency in the CLI only, with reload injected by
the CLI so the core keeps emitting a clean fragment. It was held on the grounds that a watch
mode over a mediocre preview just shows mediocrity faster; the preview is not mediocre, so
decide it on its own merits.

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

## CI — planned, not built

There is no `.github/workflows` yet. Agreed 2026-08-15 as the next ABI work item, and
**deferred to a Windows machine**, because Julian's immediate focus is publishing on the
Windows platform and the Windows leg is the only genuinely fiddly part.

### Why it is urgent rather than tidy

`tests/smoke.c` is the **only** check that catches the hand-written header drifting from the
compiled library, and it deliberately is not part of `cargo test`. So today it runs when
someone remembers. Once a NuGet package ships, consumers link against that header. The
`template`-as-a-parameter-name bug proves the point: it shipped in 0.1, made the header
uncompilable from C++ despite its `extern "C"` guards, and survived until August because
nothing checked it.

### The core job

- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
  Golden fixtures ride along, so a silently regenerated `.bin` fails the run.
- **Build and run `tests/smoke.c`.** The whole reason for the workflow.
- **ASan over `smoke.c`**, which is what validates the allocation contract.
- **Compile the header as C++** (`c++ -fsyntax-only` over a TU that only includes it).
  Cheap, and it is the check that was missing.
- An MSRV job on 1.85. `rust-version` claims it and nothing currently verifies the claim.

### Matrix: ubuntu, macos, windows — and what it is really testing

There are **no platform branches in the Rust at all** (verified: no `cfg(target…)`,
`cfg(windows)`, or `cfg(unix)` in any of the three crates). So the matrix is not testing
library logic. It tests the C toolchain integration and artifact naming, which is precisely
what breaks for a consumer rather than for us.

**The Windows leg is the work.** Everything in "Commands" above assumes Unix:

- there is no `cc` — use `cl.exe` from a Developer Prompt, or `clang` from LLVM;
- the staticlib is `tk_mdpos.lib`, not `libtk_mdpos.a`;
- the cdylib is `tk_mdpos.dll` with **no `lib` prefix**;
- linking the staticlib needs the usual Windows system libs (`ws2_32`, `userenv`,
  `advapi32`, `ntdll`, `bcrypt` — `rustc --print native-static-libs` reports the real list
  for the target, so ask it rather than guessing).

A starting point, **unverified — it has never been run on Windows**, so treat the first run
as debugging rather than as a regression:

```
cargo build -p tk-mdpos-ffi
cl /W4 /I tk-mdpos-ffi\include tk-mdpos-ffi\tests\smoke.c ^
   target\debug\tk_mdpos.lib %EXTRA_LIBS% /Fe:smoke.exe
smoke.exe
```

Confirm the artifact names against `target\debug\` before assuming them; do not copy the
Unix command and translate it by eye.

### The NuGet release job

**RIDs are decided (Julian, 2026-08-15): `win-x64` and `linux-x64`. That is the whole
claim.** `osx-*` and `android-*` are deliberately not shipped — do not add them because a
runner could produce them, and do not describe the package as supporting a platform it does
not carry a binary for. Revisit only if someone asks for one.

**Neither target can be built on the current development machine**, which is macOS arm64.
`x86_64-pc-windows-msvc` needs the MSVC toolchain, and `x86_64-unknown-linux-gnu` needs a
cross linker or a container. This is the argument for the release job existing at all — but
note it does **not** require acquiring x86 hardware: GitHub's `windows-latest` and
`ubuntu-latest` runners are both x86-64, so each target builds *natively* on its own runner
and there is no cross-compilation anywhere in the workflow. Keep it that way; a native build
per runner is why this job stays simple.

Consequence for local work: the FFI artifacts you can build and test on the Mac are
`aarch64-apple-darwin` only. That is fine for `smoke.c`, ASan and TSan, which check the
contract rather than the shipped binary — but it means **the artifacts that actually ship
have never been run on this machine**, and the packaged-`.nupkg` check below is the only
thing that exercises them.

Include a job that runs the **packaged**-crate check, not `cargo publish --dry-run`:

```
cargo package -p tk-mdpos
cd target/package/tk-mdpos-<version>/ && cargo test
```

The dry-run passed cleanly on 0.1.0 while three defects shipped, because its verify step
only builds the lib — it never builds or runs tests and never checks that licence or readme
files exist. Same trap, and NuGet's `runtimes/{rid}/native/` layout fails the same way: packs
clean, breaks on the consumer's machine, no build error.

For the .NET package the equivalent check is to **consume the packed `.nupkg` from a clean
throwaway project on each RID claimed** — `win-x64` on a Windows runner, `linux-x64` on a
Linux one — and actually call `tk_mdpos_render` through P/Invoke. `dotnet pack` succeeding
proves nothing about whether the native asset resolves. Since the shipped binaries cannot be
built or run on the development Mac at all, this job is the *only* place they are ever
executed before a user gets them.

`DllImport("tk_mdpos")` resolves `tk_mdpos.dll` and `libtk_mdpos.so` automatically on modern
.NET, so no per-platform naming is needed in the C# wrapper. That is the `tk-` convention
reaching into the C ABI paying off.

### Two things deliberately left out

- **`cargo fmt --check`.** The repo is **not currently rustfmt-clean** — `emit/escpos.rs` has
  at least two pre-existing diffs (import ordering and the `GS ( k` header call), most likely
  from edition 2024's style changes. Adding the gate fails the first run on untouched code.
  Do the reformat as its own commit first — formatting cannot change emitted bytes, and the
  golden fixtures prove that — then enable the gate. Julian has not signed off on that diff.
- **TSan on every push.** It needs nightly plus `-Zbuild-std`, which rebuilds core and std
  and costs minutes. Put it on a weekly schedule or manual dispatch. ASan is the one that
  belongs on every push.

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
