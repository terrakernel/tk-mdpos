# tk-mdpos

Turn a formatted template string into ESC/POS bytes.

```rust
tk_mdpos::render(template: &str, profile: &Profile) -> Result<Vec<u8>, Error>
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

```toml
[dependencies]
tk-mdpos = "0.2"
```

The CLI and the C ABI are not published. Both live in this repository and are built from it:

```sh
cargo install --git https://github.com/terrakernel/tk-mdpos tk-mdpos-cli
```

`tk-mdpos-ffi` was previously held back until the layout engine had been verified against
real hardware, since an ABI is a compatibility anchor. That verification has happened, so
it is now unpublished by choice rather than by gate — the C ABI is stable and usable from
this repository.

## Library

```rust
use tk_mdpos::Profile;

let template = std::fs::read_to_string("receipt.tmpl")?;   // or a database column
let profile = Profile::epson_80mm();

let bytes = tk_mdpos::render(&template, &profile)?;
send_to_printer(&bytes)?;
```

Three entry points, all sharing the same parse and layout passes:

```rust
tk_mdpos::render(&t, &p)?   // -> Vec<u8>   ESC/POS bytes
tk_mdpos::preview(&t, &p)?  // -> String    monospace, for showing a user or a test
tk_mdpos::to_ops(&t, &p)?   // -> Vec<Op>   the IR, for tooling or a custom backend
```

The profile is a plain struct:

```rust
use tk_mdpos::{Font, Profile};

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
| `{qr DATA}` | QR symbol on its own line. Honors the current justification. |
| `{qrmod N}` | QR module size in dots, 1–16. Sticky. Default 6. |
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

## QR codes

```
{center}
Scan untuk membayar
{qr 00020101021226610014COM.EXAMPLE.WWW...6304ABCD}
```

The **printer** generates the symbol — this crate never encodes one, which is why it still
has no dependency beyond `unicode-width`. What it does compute is the finished symbol's
width, so a code too wide for the paper is rejected rather than printed clipped and
unscannable.

Four things to know, none of which are worked around:

- **Error correction is fixed at level M.** Right for payment codes and assumed by QRIS.
  There is no knob.
- **Sizing is an upper bound.** Capacity is calculated for byte mode, the most expensive
  encoding; the printer may pick a cheaper one for a numeric payload and produce a smaller
  symbol than predicted. So a template within a few dots of the paper edge may be rejected
  even though it would have fit. The error is always in that direction, never the other.
- **The quiet zone counts.** Epson prints the symbol at its bare module dimensions, so the
  4-module margin per side is included in the width check.
- **Payloads are UTF-8 and bypass the code page.** QR byte mode carries opaque bytes, so a
  `{qr}` may contain characters a text line currently cannot (see **Status**).

`{size}` does not scale a QR — magnification is a text command and has no effect on the
symbol. Use `{qrmod}`.

## Images and logos

There is no `{image}` directive, and that is a decision rather than a gap. Images go through
`{raw}`:

```
{center}
{raw 1D 76 30 00 09 00 50 00 FF FF ... }
```

That is `GS v 0` — the raster bit image command — followed by the bitmap, one bit per pixel,
MSB leftmost, row-major. You build it; this crate passes it through untouched.

The reasoning: a bitmap cannot live in a template string. A full-width 80mm logo 150 rows
tall is 10,800 bytes, and the format exists so that layout sits in a database row a human
can edit. Nor will this crate decode PNGs or dither — converting a logo to 1-bit is image
processing, it needs a dependency, and whether a gradient dithers acceptably is a judgment
someone has to make by looking at the paper. That belongs with you, not here.

So the division is: **you pre-process, we pass through.** Concretely, you own

- converting to 1 bit per pixel, including any thresholding or dithering,
- padding the width to a multiple of 8, since `GS v 0` counts width in bytes,
- building the `xL xH yL yH` header to match the data you actually emit,
- deciding how many bytes your printer and transport will tolerate.

`{raw}` bytes are **opaque to the engine by design**. It will not check that the image fits
the paper, will not position it, will not draw it in the preview, and will not catch an
`xL`/`xH` header that disagrees with the data length — the classic ESC/POS image bug, which
prints as a skewed diagonal smear and reads as a hardware fault. Nothing here can see it.

**Justification applies.** `GS v 0` prints through the line buffer, so `ESC a` is what
centers an image, and `{raw}` blocks are justified like any other:

```
{center}
{raw 1D 76 30 ...}          <- centered
```

This is the one thing the engine does to a `{raw}` payload beyond passing it through. It
does not inspect the bytes; it only makes sure the device is in the state the template
asked for before they arrive.

On size: `render` hands back a `Vec<u8>`, so if you need a ceiling, check `bytes.len()`
against it in the code that knows your transport. There is deliberately no limit in the
library — it has no basis for picking a number, and a 58mm printer on Bluetooth and an 80mm
on ethernet are not the same problem.

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

## Interpolating data

**This library has no data binding, and will not grow any.** It renders a finished string.
Building that string — looping over line items, formatting currency, formatting dates — is
your application's job, in whatever language it is already written in.

That is a deliberate limit rather than a missing feature. Mustache and Handlebars are data
*binding* with no concept of bold or centre; once the caller has interpolated the data, a
template with no tags left in it is just a string. There is nothing for mdpos to add.

```rust
let mut tmpl = String::from("{cols 24,10:r,12:r}\n");
for item in &order.items {
    tmpl += &format!("{} | {} x {} | {}\n",
        escape(&item.name), item.qty, money(item.price), money(item.total));
}
```

### Escape the values, never the template

Interpolated values become template *source*. A product genuinely named
`Nasi Goreng | Spesial` turns a three-cell row into four and the render fails with
`ColumnCountMismatch`; a name containing `**` silently toggles bold.

The parser has one escape rule — `\` makes the next character literal — so the guard is
short:

```rust
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '|' | '*' | '_' | '{') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
```

| Value | Escaped | Prints as |
|---|---|---|
| `Nasi Goreng \| Spesial` | `Nasi Goreng \\\| Spesial` | `Nasi Goreng \| Spesial` |
| `**PROMO** Ayam` | `\*\*PROMO\*\* Ayam` | `**PROMO** Ayam` |
| `Kopi_Susu__Gula` | `Kopi\_Susu\_\_Gula` | `Kopi_Susu__Gula` |
| `{cut} Es Teh` | `\{cut} Es Teh` | `{cut} Es Teh` |

Two things escaping does not cover:

- **A value that is the entire content of a line and consists only of dashes** becomes a
  full-width rule. If that is reachable, prefix the line with a backslash: `\---` prints
  three dashes.
- **Leading and trailing whitespace is stripped** from every line and cell, so a value that
  depends on it will lose it. Use `\ ` for a deliberate leading space.

If you find yourself copying `escape` into a third place, that is the signal to make it a
small crate of its own — logic-less, escaping by default. Not part of this one.

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

## C ABI

`tk-mdpos-ffi` exposes the same contract across a flat C ABI, building `libtk_mdpos.a` and
`libtk_mdpos.dylib`/`.so`. The header is `tk-mdpos-ffi/include/tk_mdpos.h`.

```c
TkMdposProfile profile = tk_mdpos_profile_epson_80mm();
TkMdposBuf out;

if (tk_mdpos_render((const uint8_t *)tmpl, strlen(tmpl), &profile, &out) == TK_MDPOS_OK) {
    fwrite(out.ptr, 1, out.len, printer);
} else {
    fprintf(stderr, "mdpos: %s\n", (const char *)out.ptr);
}
tk_mdpos_free(out);   // required in both branches
```

Three rules:

1. **Every buffer must go back to `tk_mdpos_free`**, including the ones returned alongside an
   error. Rust allocated it and only Rust may release it — never `free()`.
2. **Buffers are NUL-terminated at `ptr[len]`, and `len` excludes that byte.** ESC/POS
   output contains embedded zeros (`GS V 66 0` ends in one), so `%s` truncates the output —
   but can never read past the allocation. Use `len`.
3. **Errors come back as a message, not just a code.** Template errors carry their source
   line and that text is for whoever edits the template.

Every entry point is wrapped in `catch_unwind`, since a panic unwinding across a C frame is
undefined behaviour. That requires `panic = "unwind"`, which the workspace pins.

```sh
cargo build -p tk-mdpos-ffi --release
cc -Itk-mdpos-ffi/include tk-mdpos-ffi/tests/smoke.c target/debug/libtk_mdpos.a -o smoke && ./smoke
```

That C smoke test is what verifies the header still matches the compiled ABI; the Rust
tests cannot catch header drift.

## Android and iOS

Nothing below has been built or verified yet — no cross targets are installed and there are
no Kotlin or Swift wrappers in this repo. This is the path, with the landmines marked.

For size reference, the host build with `lto = "fat"`, `opt-level = "z"` and stripped comes
to **311 KB** exporting **7 symbols**. There are no callbacks, no threads, and no I/O, which
is what makes this straightforward on both platforms.

### Android

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# NDK r27 or newer; cargo-ndk writes straight into the Gradle layout
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o app/src/main/jniLibs build --release -p tk-mdpos-ffi
```

That produces `libtk_mdpos.so` per ABI and Gradle packages it automatically. Bind it with
either a hand-written JNI shim (`extern "system" fn Java_...`, faster at runtime) or JNA
against the existing C ABI (faster to get working, adds a dependency).

Then hand the bytes to the printer. On the vendor handhelds this is the whole reason the
core is sans-IO:

```kotlin
val bytes: ByteArray = mdposRender(template, profile)
sunmiPrinterService.sendRAWData(bytes, null)   // Sunmi, iMin, Telpo — same shape
```

> **16KB page size.** Android 15+ requires 16KB-aligned native libraries, and Rust does not
> do it for you:
>
> ```
> -C link-arg=-Wl,-z,max-page-size=16384
> ```
>
> The trap is that omitting it **passes local testing and fails Play review**. Check the
> current Play policy rather than trusting this note — the deadlines here have moved more
> than once.

### iOS

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build -p tk-mdpos-ffi --release --target aarch64-apple-ios
```

Package `libtk_mdpos.a` plus `include/tk_mdpos.h` and a `module.modulemap` as an **XCFramework**,
then consume it as a SwiftPM `binaryTarget` or drop it into Xcode. Swift calls the C ABI
directly — no shim needed. Bitcode has not been required since Xcode 14.

> **MFi.** The blocker on iOS is hardware, not code. Bluetooth *Classic* printers require
> the printer vendor to be MFi-certified; BLE via CoreBluetooth has no such gate. Settle
> this before writing any Swift.

### Three things to get right

1. **Wrap the buffer in something with a destructor.** `tk_mdpos_free` must run on every path,
   including errors. A Kotlin `use {}` or a Swift type with `deinit` makes that structural
   instead of a thing you have to remember.
2. **Keep `panic = "unwind"`.** It costs roughly 100 KB against `panic = "abort"`, but
   panics unwinding through a JNI or Swift frame are undefined behaviour and `catch_unwind`
   cannot work without it. The workspace pins it deliberately.
3. **Error messages are English UTF-8 carrying template line numbers.** They are written for
   whoever edits the template, not for the customer holding the receipt. Surface them in
   your admin UI and log them; do not put them on screen at the till.

If you end up hand-writing both a Kotlin and a Swift wrapper, it is worth a few minutes
looking at UniFFI, which generates both from the Rust API. It would largely replace this
crate rather than sit on top of it, and for a surface of three functions that matter the
hand-written C ABI is probably still the better trade — but make that call deliberately.

## Clone printers

**Genuine Epson is the target.** Clone printers — Xprinter, Rongta, EPPOS, Gainscha — have
no specification, and their deviations cluster in exactly the places that cannot be probed
for: cut variants, `ESC $` handling, and native QR. Chasing them means an open-ended bug
surface and compatibility claims nobody can falsify, so this crate emits the Epson command
and stops. No fallbacks, no per-vendor branches.

`{raw HEX}` remains an escape hatch, and it is not a hack — it means a vendor-specific cut
or drawer kick never blocks a release. But it puts the quirk in the caller's hands
deliberately; it is not a claim of clone support.

Note that "standard ESC/POS" is not a standard. It is Epson's proprietary command set,
copied to varying degrees, with no spec body and no certification. Compatibility claims
here are meant to be falsifiable: print a test template and compare.

## Format stability

Templates may declare `{v 1}`. The *string* carries the compatibility promise, not the
crate version — the engine may be rewritten freely, but a v1 template must render
identically in perpetuity. If syntax changes could drag deployed templates back into the
redeploy cycle, the entire premise of this library collapses.

## Status

The pipeline is complete end to end, covered by unit tests and golden fixtures, and
**verified on real hardware**: an Epson TM-T82X over port 9100, printing right-aligned
prices flush to the margin, a double-width centered header, wrapped product names, a
magnified total row, a scannable payment QR, and a clean cut — with no recompilation
between template edits.

Known limitations:

- **Non-ASCII text is rejected**, not mangled. The CP437 high range (0x80–0xFF) is not
  mapped yet, so `café` returns an error rather than printing wrong. Width *measurement* is
  already `unicode-width` throughout. QR payloads are exempt — they go out as UTF-8.
- **QR error correction is fixed at M**, and symbol sizing is a deliberate upper bound.
  See *QR codes* above.
- One built-in profile (80mm, Font A, Epson). No profile registry or TOML loading.
- **Genuine Epson is the target.** Clone printers are not chased; see *Clone printers*.

Out of scope, and not merely deferred: WASM, barcodes, images, data interpolation, and the
Star, ZPL, and TSPL dialects. Each gets decided on its own merits rather than added because
it is adjacent to something that already exists.

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

Copyright (c) 2026 TERRAKERNEL PTE. LTD.

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
