# mdpos

Turn a formatted template string into ESC/POS bytes.

```rust
mdpos::render(template: &str, profile: &Profile) -> Result<Vec<u8>, Error>
```

That is the entire public contract.

## Why

Every other ESC/POS library is a command builder — you call `.bold().text().align()` from
your application. That compiles receipt layout into the binary, so changing a footer means
a rebuild, a redeploy, and a test cycle.

mdpos moves layout into a **string**. It can live in a database row, a config field, or a
text area. Your application forwards it; the engine figures out the rest. Layout changes
stop being releases.

The differentiator is the layout engine, not the parser: right-alignment computed in dots,
a grid that tracks magnification, per-column overflow policy, and widths measured with
`unicode-width`.

## Example

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

`mdpos --preview receipt.tmpl`:

```
               TOKO MAJU
                Jl. Sudirman 42
------------------------------------------------
Nasi Goreng         2 x 25.000      50.000
Es Teh Manis        3 x  5.000      15.000
------------------------------------------------
TOTAL                               65.000
```

## Install

Not published yet. Use a path or git dependency:

```toml
[dependencies]
mdpos = { git = "https://github.com/terrakernel/mdpos" }
```

## Library

```rust
use mdpos::Profile;

let template = std::fs::read_to_string("receipt.tmpl")?;   // or a database column
let profile = Profile::epson_80mm();

let bytes = mdpos::render(&template, &profile)?;
send_to_printer(&bytes)?;
```

Three entry points, all sharing the same parse and layout passes:

```rust
mdpos::render(&t, &p)?   // -> Vec<u8>   ESC/POS bytes
mdpos::preview(&t, &p)?  // -> String    monospace, for showing a user or a test
mdpos::to_ops(&t, &p)?   // -> Vec<Op>   the IR, for tooling or a custom backend
```

The profile is a plain struct:

```rust
use mdpos::{Font, Profile};

let narrow = Profile { width_dots: 384, ..Profile::epson_80mm() };  // 58mm, 32 columns
let dense  = Profile { font: Font::B,   ..Profile::epson_80mm() };  // 64 columns
```

`profile.columns()` gives characters per line — always derived from `width_dots` and the
font, never hardcoded. `columns_at(2)` gives it under `{size 2x2}`.

## CLI

```sh
mdpos receipt.tmpl > out.bin      # ESC/POS bytes to stdout — always redirect
mdpos --preview receipt.tmpl      # monospace preview
cat receipt.tmpl | mdpos -        # read from stdin
```

The CLI uses `Profile::epson_80mm()`. There is no profile flag yet.

## Syntax

| Directive | Meaning |
|---|---|
| `{v 1}` | Format version. Optional, but must come first if present. |
| `{left}` `{center}` `{right}` | Justification. Sticky until changed. |
| `{size WxH}` | Character magnification, 1–8 each. Sticky. Halves the grid at 2x. |
| `{cols A,B:r,C:c}` | Column widths in characters. `:l` default, `:r` right, `:c` center. Sticky. |
| `{/cols}` | Leave column mode. |
| `---` | Full-width rule. Three or more dashes, alone on the line. |
| `{feed N}` | Feed N lines. |
| `{cut}` | Partial cut. |
| `{raw 1D564200}` | Hex passthrough. Spaces allowed: `{raw 1D 56 42 00}`. |
| `**text**` | Bold. |
| `__text__` | Underline. |

Directives may stand alone on a line or prefix one — `{center}{size 2x2}TOKO MAJU` works.

While a `{cols}` spec is active, lines split on `|` and **the cell count must match the
spec exactly**. Outside column mode, `|` is ordinary text.

Every document is self-contained: it begins with `ESC @`, ends with a feed and a cut, and
assumes nothing about the printer's prior state. A thermal printer is a stateful
interpreter — leave emphasis on and the *next* receipt prints bold until someone power
cycles it.

## Four things that will bite you

**Whitespace is stripped from both ends of every line and cell.** `Nasi Goreng    | 50.000`
and `Nasi Goreng|50.000` are identical. This is deliberate: templates live in database rows
and text areas that do not preserve trailing spaces, so alignment is *stated* with `:r`,
never implied by padding. If you genuinely need a leading space, escape it — `\ Total`.

**`\` is the only escape rule.** It makes the next character literal: `\|`, `\*`, `\_`,
`\{`, `\\`.

**Column widths are in *current* characters.** Under `{size 2x2}` a width of 20 means 20
double-width characters — 40 base cells. So `{cols 20,10:r,12:r}` totals 42 and is fine at
1x, but is rejected at 2x, where only 24 columns exist.

**Right-aligned columns never wrap.** Overflow is an error, because a wrapped total prints
as two lines that read as two different numbers:

```
line 3: "1.250.000" overflows right-aligned column 2 (width 6); right-aligned columns never wrap
```

Left and centered columns wrap, with continuation lines returning to the column's own start.

## Errors

`Error` implements `Display` and `std::error::Error`, and carries the 1-based source line.
Templates are edited by hand with no compiler in between, so surface the message verbatim
to whoever edits them.

## Sans-IO

The crate does not know what a printer is. No sockets, no serial ports, no USB, no
filesystem, no async runtime. It produces bytes; delivering them is the caller's job.

This is not minimalism. Printer transport is a platform tarpit — the Windows spooler,
`/dev/usb/lp0`, BLE GATT, Bluetooth RFCOMM, and vendor AIDL services are all different
problems, and the largest Android hardware (Sunmi, iMin, Telpo) exposes nothing but
`sendRAWData(byte[])`. Producing bytes is the only thing that works everywhere.

```sh
mdpos receipt.tmpl > out.bin

cat out.bin > /dev/usb/lp0         # Linux USB
nc 192.168.1.50 9100 < out.bin     # network / most WiFi printers
```

From an application: write the bytes to a serial port, a TCP socket on port 9100, or hand
them to a vendor SDK. Queueing, chunking, retries, job atomicity, and paper-out status
polling all belong to the caller or to a separate crate.

## Clone printers

`{raw HEX}` is an escape hatch, not a hack. Clone printers — Xprinter, Rongta, EPPOS,
Gainscha — have no specification, and their deviations cluster in cut variants, `ESC $`
handling, and native QR. That is unfixable in principle, so `{raw}` means a vendor-specific
cut or drawer kick never blocks on a release.

Note that "standard ESC/POS" is not a standard. It is Epson's proprietary command set,
copied to varying degrees, with no spec body and no certification. Compatibility claims
here are meant to be falsifiable: print a test template and compare.

## Format stability

Templates may declare `{v 1}`. The *string* carries the compatibility promise, not the
crate version — the engine may be rewritten freely, but a v1 template must render
identically in perpetuity. If syntax changes could drag deployed templates back into the
redeploy cycle, the entire premise of this library collapses.

## Status

v0.1. The pipeline is complete end to end and covered by unit tests and golden fixtures.

Known limitations:

- **Non-ASCII text is rejected**, not mangled. The CP437 high range (0x80–0xFF) is not
  mapped yet, so `café` returns an error rather than printing wrong. Width *measurement* is
  already `unicode-width` throughout.
- One built-in profile (80mm, Font A, Epson). No profile registry or TOML loading.
- Not yet verified against real hardware.

Out of scope for v0.1, and not merely deferred: FFI and C ABI, WASM, Android bindings, QR
codes, barcodes, images, data interpolation, and the Star, ZPL, and TSPL dialects. None of
them can be evaluated sensibly until the layout engine has proven itself in print.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
UPDATE_GOLDEN=1 cargo test --test golden    # regenerate fixtures, then read the diff
```

Golden fixtures live in `tests/golden/` and are structured as if they will be published —
they are the seed of a conformance corpus and of a customer-facing compatibility test. Both
backends snapshot from the same input, which is what keeps the preview honest about what
the bytes will do.

A changed `expected.bin` is a v1 compatibility break until proven otherwise. Inspect the
diff by hand.

## License

MIT OR Apache-2.0.
