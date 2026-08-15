# mdpos — Project Instructions

Handoff document. Read fully before writing code.

> **Status, 2026-08-15.** v0.1 shipped and **§12 is met** — a hand-edited template printed
> correctly on an Epson TM-T82X. Since then: a C ABI (`tk-mdpos-ffi`), QR symbols, and images
> via `{raw}`. Each was out of scope in §2 as originally written and was taken on deliberately;
> the sections below are updated to say so rather than left to disagree with the repository.
>
> The §1 constraints have not moved and are not expected to. Where a decision here was
> superseded, the original reasoning is kept — it is usually still the reason the replacement
> looks the way it does. `CLAUDE.md` carries the working notes; this file carries the design.

---

## 0. What this is

A Rust library that turns a **formatted template string** into **ESC/POS bytes**.

```
(template: &str, profile: &Profile) -> Result<Vec<u8>, Error>
```

That is the entire public contract. Nothing else is load-bearing.

**Why it exists:** every existing ESC/POS library (`escpos-rs`, `python-escpos`, `escpos-php`, `node-thermal-printer`) is a *command builder* — you call `.bold().text().align()` in host code. That means receipt layout is compiled into the application, and changing a footer requires a rebuild, a redeploy, and a test cycle.

mdpos moves layout into a **string** that can live in a database row, a config field, or a text area. The application forwards the string; the engine figures out the rest. Layout changes stop being releases.

**The differentiator is the layout engine, not the parser.** Confirm the gap still exists (see §9) before building.

---

## 1. Non-negotiable design constraints

These were settled after long deliberation. Do not relitigate without a concrete reason.

### 1.1 Sans-IO. Absolutely no transport in the core.

The core crate must not know what a printer is. No `tokio`, no `serialport`, no USB, no sockets, no file handles. Pure function, deterministic, ideally no deps beyond `unicode-width`.

Rationale:
- Printer I/O is a platform tarpit (Windows spooler vs `/dev/usb/lp0` vs BLE GATT vs Bluetooth RFCOMM vs vendor AIDL). Every one of those is host-language territory.
- On Android, the biggest target hardware (Sunmi, iMin, Telpo) exposes only `sendRAWData(byte[])` through a vendor AIDL service. Rust has no path to the hardware at all. Producing bytes is the *only* thing that works there.
- A pure `char* in, uint8_t* out` function is the cleanest possible thing to push across a flat C ABI later. No callbacks, no lifetimes, no GC interaction.

Anything about queueing, chunking, retry, `DLE EOT` status polling, or job atomicity is **out of scope**. It belongs to the caller or to a separate future crate.

### 1.2 Keep an `Op` IR between parser and emitter.

Never compile template text directly to bytes. This is ten lines of discipline and it is the entire insurance policy for everything downstream (preview backend, dialect support, vendor-SDK backend, profile-driven width).

### 1.3 Format version marker from day one.

Templates start with `{v 1}` (optional in v0.1, defaulted to 1; required to be *honored* forever after). The engine may be rewritten freely; v1 templates must render identically in perpetuity.

Rationale: the string is the public API. It carries semver, not the crate. If syntax changes drag deployed templates back into the redeploy cycle, the whole premise collapses.

### 1.4 Profile is a separate input, not a template constant.

Paper width and dialect cannot be inferred from a template. `(string, profile) -> bytes`, always. Same template must render 58mm and 80mm.

---

## 2. v0.1 scope — two weeks, Rust only

Ship small. Everything below is deliberate.

**In:**
- One hardcoded profile: 80mm, 576 dots, Font A, 48 cols, Epson dialect. `Profile` exists as a struct; no TOML loading yet.
- Syntax subset: `{center}` `{left}` `{right}`, `{size WxH}`, `{cols ...}`, `**bold**`, `---`, `{feed n}`, `{cut}`, `{raw HEX}`.
- Two backends: ESC/POS bytes, and plaintext monospace preview.
- CLI: `mdpos template.txt > out.bin`, and `mdpos --preview template.txt`.
- Golden-file tests.

**Out — not "later in the plan," not decided:**
- FFI / C ABI / bindings
- WASM
- Android
- QR codes, barcodes, bitmaps/logos
- Profile registry / TOML
- Data interpolation of any kind (see §8)
- Any web service
- Conformance corpus as a published artifact
- Star / ZPL / TSPL dialects

Rationale: none of the deferred items can be evaluated until the layout engine exists and is known to be good. If the layout engine turns out ugly, that's two weeks lost instead of a cross-language ABI built on sand.

### 2.1 What has since moved (2026-08-15)

The gate above was the §12 print, and it passed. The list is therefore no longer a wall; it is a set of open questions to decide one at a time. Three have been decided:

| Item | Status | Note |
|---|---|---|
| FFI / C ABI | **Built** | `tk-mdpos-ffi`, taken on ahead of the gate at explicit direction. The concern raised at the time — that an ABI anchors the `Op` IR before a print could prove it right — is discharged: the print happened and the engine was right. |
| QR codes | **Built** | `{qr}` / `{qrmod}`, one `Op` variant, one command family. Cheap because the *printer* encodes the symbol; only its printed width is computed here. See §4 and §13. |
| Bitmaps / logos | **Decided: no directive** | Images go through `{raw}` as `GS v 0` raster. The caller pre-processes. See §13. |

Still out, and each to be argued on its own merits rather than by adjacency: WASM, barcodes, profile registry / TOML, data interpolation, web services, published conformance corpus, Star / ZPL / TSPL.

The original rationale still governs the remainder — but note its premise has been consumed. "None of this can be evaluated until the layout engine is known to be good" was a statement about a specific unknown, and that unknown is now resolved. It is not a general-purpose argument for saying no.

---

## 3. Architecture

```
template string
      ↓  parse          (~300 lines, the cheap part)
   Block AST
      ↓  layout         (THE PRODUCT — all difficulty lives here)
   Vec<Op>              (IR — device-independent, width-resolved)
      ↓  emit           (per-dialect backend)
   Vec<u8>
```

The layout pass consumes the `Profile`. The emitter is nearly mechanical.

### 3.1 The IR

```rust
pub enum Op {
    Text(String),
    Emphasis(bool),
    Underline(bool),
    Justify(Align),
    Size { w: u8, h: u8 },        // 1..=8
    AbsPos(u16),                  // dots from left margin
    Feed(u8),                     // lines
    Cut(CutKind),
    Raw(Vec<u8>),
    Qr { data: String, module: u8 },   // added v0.2
    // later, if ever: Barcode {..}
}

pub enum Align { Left, Center, Right }
pub enum CutKind { Partial, Full }
```

Note `Row`/`Cell` do **not** appear in the IR. Columns are resolved during layout into `Text` + `AbsPos` sequences. The IR is post-layout and width-independent by construction.

`Qr` obeys the same rule. It survives into the IR only because the *printer* renders the symbol from the data — layout has already resolved the one width question, namely whether it fits the paper. There is no `Image` variant for the same reason in reverse: a bitmap has no representation the engine could reason about, so it stays opaque inside `Raw`.

### 3.2 Profile

```rust
pub struct Profile {
    pub dialect: Dialect,         // v0.1: Epson only
    pub width_dots: u16,          // 576 = 80mm, 384 = 58mm
    pub font: Font,               // A: 12 dots/char, B: 9 dots/char
    pub code_page: CodePage,      // v0.1: Cp437
    pub supports_partial_cut: bool,
}
```

Columns are *derived*: `width_dots / font.char_width_dots`. Never hardcode 48.

---

## 4. Syntax v1

Block directives occupy their own line or prefix a line. Inline formatting is a deliberately tiny markdown subset.

```
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
```

| Directive | Meaning |
|---|---|
| `{v N}` | Format version. Optional in v0.1, assumed 1. |
| `{left}` `{center}` `{right}` | Justification, sticky until changed. |
| `{size WxH}` | Character magnification, 1..8 each. Sticky. |
| `{cols A,B:r,C:r}` | Column spec in **characters**. `:r` right-align, `:l` default, `:c` center. Sticky until next `{cols}` or `{/cols}`. |
| `---` | Full-width separator, filled with `-`. |
| `{feed N}` | Feed N lines. |
| `{cut}` | Partial cut. Emitter auto-prepends feed if profile needs it. |
| `{qr DATA}` | QR symbol on its own line, honoring current justification. Added v0.2 — see §13. |
| `{qrmod N}` | QR module size in dots, 1..16. Sticky. Default 6. Added v0.2. |
| `{raw 1D564200}` | Hex passthrough. Load-bearing escape hatch — see §7. Also the image mechanism, see §13. |
| `**text**` | Bold (`ESC E`). |
| `__text__` | Underline (`ESC -`). |

While a `{cols}` spec is active, lines are split on unescaped `|`. Escape a literal pipe as `\|`.

**Design note:** the table is the *central* primitive here, not an afterthought as in GFM. Column widths are explicit because they cannot be inferred — this is the single reason CommonMark was rejected as the input format. See §8.

---

## 5. The layout engine — where the difficulty actually is

### 5.1 Right-alignment is arithmetic in dots, not space padding

Use `ESC $ nL nH` (absolute horizontal position, `nL + nH*256` dots from left margin). It is font-width independent and survives font A/B switches mid-line. Space-padding breaks the moment magnification or font changes.

`ESC D ... NUL` (tab stops) exists as a fallback for clones that mishandle `ESC $`, but is not v0.1.

### 5.2 Magnification mutates the grid mid-document

`{size 2x2}` halves the effective column count: 48 → 24. The line breaker must track *current* character width, not a document constant. This is the most common source of subtly wrong output.

### 5.3 Width is `unicode-width`, never `str::len()` or `chars().count()`

Use the `unicode-width` crate. Even for a mostly-ASCII market, customer names and addresses will eventually hand you something that isn't.

### 5.4 Per-column overflow policy

Long product names are the common case, not an edge case.

- Left/default columns: **wrap with hanging indent**, continuation lines aligned to the column's start position.
- Right-aligned columns (prices, totals): **never wrap**. Truncate or error. A wrapped total is worse than a rejected template.
- Default policy in v0.1: wrap for `:l`/`:c`, error for `:r`. Revisit once real templates exist.

### 5.5 State is sticky and global on the device

The printer is a stateful interpreter with a line buffer, not a page-description consumer. Forget to reset bold and every subsequent receipt is bold until power cycle.

Therefore every rendered document must be **self-contained**:
- Always begins with `ESC @` (init, reset all state).
- Always ends with feed + cut.
- Assumes nothing about prior device state.

This makes a lost or duplicated document unable to corrupt the next one.

---

## 6. ESC/POS commands needed for v0.1

| Command | Bytes | Notes |
|---|---|---|
| Initialize | `1B 40` | `ESC @`. Always first. |
| Justify | `1B 61 n` | n = 0 left, 1 center, 2 right |
| Char size | `1D 21 n` | `GS !`. High nibble = width−1, low nibble = height−1. `0x11` = 2x2. |
| Bold | `1B 45 n` | `ESC E`, n = 0/1 |
| Underline | `1B 2D n` | `ESC -`, n = 0/1/2 |
| Absolute pos | `1B 24 nL nH` | `ESC $`, dots from left margin |
| Select font | `1B 4D n` | `ESC M`, 0 = A, 1 = B |
| Code page | `1B 74 n` | `ESC t`. Page 0 = CP437 almost everywhere; diverges after that. |
| Feed lines | `1B 64 n` | `ESC d` |
| Partial cut | `1D 56 42 00` | `GS V 66 0` — feed-and-cut variant, the safe default |
| Full cut | `1D 56 00` | `GS V 0` |
| Line feed | `0A` | Commits the line buffer to paper |

Not needed in v0.1 but noted: `DLE EOT n` = `10 04 n`, the only real-time status query (paper out, cover open). Requires a read channel, which several transports don't have. Transport concern, not ours. (It does work on the TM-T82X over port 9100, and was used to pre-flight the §12 print — but from a test script, not from the library.)

Reference: the Epson ESC/POS Command Reference is public and thorough. Use it, not blog posts.

Added since (see §13):

| Command | Bytes | Notes |
|---|---|---|
| QR family | `1D 28 6B pL pH 49 fn ...` | `GS ( k`. Five calls: model (`fn` 65), module size (67), error correction (69), store (80), print (81). `pL + pH*256` counts bytes from `cn` onward. |
| Raster image | `1D 76 30 m xL xH yL yH d...` | `GS v 0`. Width in **bytes**, height in rows, 1bpp, MSB leftmost. Emitted by callers inside `{raw}`, never by this crate. |

---

## 7. `{raw}` is load-bearing, not a hack

Clone printers (Xprinter, Rongta, EPPOS, Gainscha — the actual installed base in Indonesian retail) have no spec. Deviations cluster in cut variants, `ESC $` handling, and native QR (`GS ( k`) being absent entirely.

This is unfixable in principle. `{raw HEX}` means a weird drawer-kick, a proprietary logo command, or a vendor-specific cut never blocks on a release. Implement it in v0.1; it costs one `Op` variant.

**Product positioning that follows from this:** support "standard ESC/POS." Note that this is not a real standard — it's Epson's proprietary command set, copied to varying degrees, with no spec body and no certification. So make the claim falsifiable: ship a compatibility test template plus an image of its expected printed output. *"Print this. If it matches, you're supported."* Non-matching printers become paid profile work, which is only economically sane if a new printer is **config, not code** — hence §1.4.

### 7.1 Superseded: clones are not chased (2026-08-15)

The above argues for `{raw}` *from* clone compatibility. That premise is withdrawn. The target is genuine Epson; cheap copies are not chased, no fallbacks or per-vendor branches are added for them, and nothing is probed at runtime.

The reasoning is the paragraph above taken to its conclusion. If deviation is unfixable in principle and no claim about it can be made falsifiable, then chasing it buys an open-ended bug surface and no defensible promise. One authoritative reference — the Epson command reference — is worth more than partial agreement with four vendors who agree with nobody, including each other.

`{raw}` survives this intact and is now needed *more*, because it turned out to be the image mechanism (§13). Its justification simply changes: not "we support clones," but "a caller sometimes needs bytes on the wire that this crate has no opinion about." Everything §7 says about it being load-bearing rather than a hack still holds.

The falsifiable-compatibility idea in the last paragraph is still the right instinct and still unbuilt. It is a better fit now than before: with one target dialect, "print this and compare" has a single correct answer.

---

## 8. Rejected alternatives (do not revisit without new information)

**Mustache / handlebars as the format.** Mustache is data *binding*; it has no concept of bold or center. With data pre-interpolated by the caller — which is the decided model — a template with no tags is just a string. Out entirely. If interpolation is ever added, it goes in a separate optional crate, stays logic-less, and escapes format metacharacters by default.

**CommonMark / GFM as the format.** Markdown's design premise is "reflow prose, author does not control layout." A receipt is the exact opposite: a fixed-width grid with hard right-alignment. The dominant idiom —

```
Nasi Goreng            2 x 25.000    50.000
```

— has no markdown primitive. GFM tables leave column widths implicit, which is the one thing that cannot be inferred. Supporting receipts would require directives for align, cut, drawer, QR, columns, and feed — ending up ~70% custom syntax with markdown along for the ride on `**bold**`. Taking only markdown's *inline* layer (which v1 does) captures the real benefit without the mismatch.

**djot.** Genuinely better fit than CommonMark — native attribute syntax, fenced divs, unambiguous grammar, a Rust impl (`jotdown`). Rejected because the familiarity argument was markdown's entire advantage, and djot has none of it. Still worth a look if the hand-rolled parser gets unwieldy.

**HTML + CSS subset.** Defensible — real browsers give free preview, `html5ever` parses it, clients already know it. Rejected because partial CSS support means preview silently diverges from print, and you spend forever explaining why `float` doesn't work. Monospace preview is more honest about the grid.

**Multiple native implementations (Java, .NET, Swift, Node).** Five implementations means five layout engines, and keeping them byte-identical for years is the actual cost. Fix a clone quirk → five PRs, five releases, five versions in the wild; six months later one is three quirks behind and nobody notices until a customer does. Team is three people. Path forward instead: Rust core + a published conformance corpus, so anyone *can* reimplement and *prove* it correct.

---

## 9. Do this before writing the parser

> **Both gates were run in July 2026 and both are closed.** ReceiptLine was evaluated and
> **not adopted**: three conflicts with settled constraints, the decisive one being that it
> encodes alignment in whitespace around the `|`, and significant trailing spaces do not
> survive a textarea, a form submit, a YAML round-trip, or most ORM-adjacent trimming — which
> is precisely where §0 says these templates will live. Its property model was mined instead
> of its grammar. The gap was also confirmed: every library checked is still builder-only.
> Full findings are in `CLAUDE.md`; do not re-run these without new information.

**Evaluate ReceiptLine.** OSS, markdown-flavored receipt description language, ESC/POS + StarPRNT output, SVG preview. It is approximately the thing designed above and has a claim to being the de facto standard in this niche. Spend an hour.

- If it fits → implement *that* grammar in Rust. "ReceiptLine for Rust" is a far easier sell than "my receipt syntax," and existing templates work.
- If it doesn't → you now have a concrete list of why, which is the actual justification for rolling your own.

Either way, mine its edge cases. It has hit the clone nonsense already.

**Also confirm the gap.** Verify that `escpos-rs`, `python-escpos`, `escpos-php`, and `node-thermal-printer` are still builder-only with no template-string layout. If one has grown a layout engine since, that changes the plan.

---

## 10. Testing

Golden files, from commit one:

```
tests/golden/
  001-basic-columns/{input.tmpl, profile.ron, expected.bin, expected.txt}
  002-double-width-grid/
  003-long-name-wrap/          (planned as 003-unicode-wrap; see below)
  004-right-align-no-wrap/     (expected.err — pins a refusal, not an output)
  005-qr-payment/              (added v0.2)
  006-qr-too-wide/             (added v0.2, expected.err)
```

Output must be byte-deterministic. Both backends (bytes + preview) snapshot from the same fixture.

Two deviations from the plan above, both deliberate:

- **`003` covers wrapping, not unicode.** The unicode fixture is blocked on the CP437 high range in the encoder: non-ASCII text is currently *rejected* rather than mangled, so no `expected.bin` can exist for it. Width measurement is already `unicode-width` throughout and is covered by unit tests. Add the fixture with the encoder table, not before.
- **A fixture may assert a refusal** via `expected.err` instead of `expected.bin`/`.txt`. Refusing a template is a feature here — §5.4 makes a wrapped total an error — so the corpus has to be able to pin refusals too, or the most important behavior in §5 would be untested.

This directory is the seed of the future conformance corpus, and eventually of the customer-facing compatibility test. Structure it as if it will be published, even though publishing is out of scope for v0.1.

---

## 11. Known external variables (context, not v0.1 work)

- **Android 15 / 16KB page size.** Play requires 16KB-aligned native libs for apps targeting Android 15+. NDK r27+ handles it, but Rust needs `-C link-arg=-Wl,-z,max-page-size=16384` explicitly. Passes local testing, fails Play review. Verify current policy before planning around it.
- **`catch_unwind` vs `panic=abort`.** FFI happened, so this is settled rather than hypothetical: `panic = "unwind"` is pinned in the workspace `[profile.release]` and every entry point is wrapped in `catch_unwind`. `panic = "abort"` would remove that net silently, forfeiting ~100KB of savings to keep it. Correct trade; do not "optimize" it back.
- **Apple MFi.** Bluetooth *Classic* printers on iOS require the printer vendor to be MFi-certified. BLE via CoreBluetooth has no such gate. Hardware-selection problem, not engineering.
- **Locked vendor SDKs.** Some cheap Chinese handhelds expose only `printText(str, size, align)` with no raw channel. The `Op` IR makes an SDK-call backend possible without raw bytes. Check for `sendRAWData` before promising support.
- **Bytes ≠ ESC/POS.** ZPL (Zebra), TSPL (label printers), Star line mode are all bytes and none are interchangeable. Emitter-level concern, handled by the IR.
- **Fiscal/tax.** Indonesia has no fiscal-printer mandate, but Jakarta Bapenda runs online monitoring for restaurant tax (PB1), and QRIS receipts have conventions. Affects required *content*, not printing. Worth verifying rather than assuming.

---

## 12. Success criterion for v0.1

A template file, edited by hand, rendered by CLI, printed on a real 80mm printer, producing a receipt with correctly right-aligned prices, a double-width header, wrapped long product names, and a clean cut — with zero recompilation between layout edits.

If that works and the layout code isn't horrible, the deferred items in §2 become worth discussing. Not before.

Build the renderer.

### 12.1 Met — 2026-08-15

Printed on an Epson TM-T82X (80mm, network, port 9100). Every clause of the criterion held: prices right-aligned flush to column 48, a 2x2 centered header, long names wrapping with the hanging indent returning to the column's own start, a magnified `TOTAL` row, and a clean partial cut clear of the tear edge. Rendered by CLI from a hand-edited file with no recompilation.

The `ESC $` arithmetic was checked by hand against the hexdump *before* printing rather than inferred from the preview — which is the habit worth keeping, since the preview is derived from the same layout pass and would have agreed with a wrong answer.

Nothing in the layout engine turned out to be wrong. That is what discharges the §2 gate and what makes the `Op` IR safe to build against.

---

## 13. Features added after this document was written

Design notes for what postdates §2. Working detail lives in `CLAUDE.md`; the decisions and their reasons are here.

### 13.1 QR — `{qr}` and `{qrmod}`

**The printer encodes; this crate only sizes.** `GS ( k` takes the payload and generates the symbol, so there is no encoder, no bitmap, and no new dependency. The only thing computed here is the finished symbol's width, because layout must reject an overflowing template rather than print a clipped, unscannable code — the same reasoning as §5.4, and for higher stakes: a clipped payment code is a lost sale, not a cosmetic defect.

Four choices, each documented rather than engineered around:

- **Error correction is fixed at M.** Correct for payment codes, assumed by QRIS, and it keeps the capacity table one column instead of four.
- **Sizing is a deliberate upper bound.** Capacities are byte-mode, the most expensive encoding, and the printer picks the mode itself — so a numeric payload may be rejected within a few dots of fitting. The error is always in the safe direction.
- **The quiet zone counts against the paper.** Epson prints the symbol at its bare module dimensions, and a code flush to the edge scans badly.
- **Payloads are UTF-8 and bypass the code page.** QR byte mode carries opaque bytes. So a `{qr}` may legally contain characters a text line currently cannot, and the payload is measured in *bytes* — the one width in the engine that is deliberately not `unicode-width`.

`GS !` does not scale a symbol, so `{size}` is a no-op on a QR. Confirmed on hardware, along with the fact that `GS ( k` prints through the line buffer — which is why `ESC a` places it and no dot arithmetic is needed.

### 13.2 Images — no directive, by decision

Images go through `{raw}` as `GS v 0` raster bytes. **There is no `{image}` directive and there should not be one.** The audience is developers, so pre-processing is theirs: 1-bit conversion, dithering, padding width to a multiple of 8, and building the `xL xH yL yH` header.

A bitmap cannot live in a template string. A full-width 80mm logo 150 rows tall is 10,800 bytes, and §0 says these strings live in database rows a human edits. Nor will this crate decode PNGs or dither — that is image processing, it needs a dependency, and whether a gradient dithers acceptably is a judgment someone has to make by looking at paper.

Three alternatives were rejected, and the reasons are more durable than the outcome:

- **NV logo slots (`{logo N}`)** — a logo in printer flash is *per-device state set at install time*, which contradicts §5.5: every document begins with `ESC @` and assumes nothing about prior device state. Same template, two printers, different receipt. Check proposals against §1 and §5.5 before making them.
- **A caller-supplied asset map** (`render_with(template, profile, assets)`) — workable without breaking the existing signature, but unnecessary API surface when a caller can prepend the bytes itself.
- **A document size limit on `Profile`** — `{raw}` cannot amplify, since two hex characters become one byte, so it is not a safety feature; and the caller already holds the returned `Vec<u8>`. Revisit only if templates become end-user-editable, where the author and the operator stop being the same person.

**Layout applies justification to a `{raw}` block and does nothing else to it.** That was a fix, not the original behavior: `{center}` directly above a `{raw}` emitted nothing, so an image sat flush left while the template asked for centering — and it only appeared to work when a printed line above happened to leave `ESC a` set. Found by printing the same image four times and comparing positions.

Everything else in a `{raw}` payload is passed through unexamined, including an `xL`/`xH` header that disagrees with the data. That is the classic ESC/POS image bug, it prints as a diagonal smear that reads as a hardware fault, and nothing here can catch it. Stated plainly in the README rather than mitigated.
