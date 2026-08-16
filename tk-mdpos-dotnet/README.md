# TerraKernel.Mdpos

Turn a formatted template string into ESC/POS receipt bytes.

```csharp
byte[] bytes = Mdpos.Render(template, PrinterProfile.Epson80mm);
```

That is the entire public contract.

## Why

Every other ESC/POS library is a command builder — you call `.Bold().Text().Align()` from
your application. That compiles receipt layout into the assembly, so changing a footer means
a rebuild, a redeploy, and a test cycle.

mdpos moves layout into a **string**. It can live in a database column, a config field, or a
text area. Your application forwards it; the engine figures out the rest. Layout changes stop
being releases.

The differentiator is the layout engine, not the parser: right-alignment computed in dots, a
grid that tracks magnification, per-column overflow policy, and widths measured by Unicode
display width rather than character count.

## Install

```
dotnet add package TerraKernel.Mdpos
```

Native binaries are carried for **win-x64** and **linux-x64**. Those are the whole claim.
Apple platforms are supported through a separate channel — an XCFramework consumed as a
Swift package — rather than through NuGet.

## Example

```csharp
using TerraKernel.Mdpos;

const string template = """
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
    """;

byte[] bytes = Mdpos.Render(template, PrinterProfile.Epson80mm);

using var client = new TcpClient("192.168.1.50", 9100);
client.GetStream().Write(bytes);
```

`Mdpos.Preview(template, PrinterProfile.Epson80mm)`:

```
               TOKO MAJU
                Jl. Sudirman 42
------------------------------------------------
Nasi Goreng         2 x 25.000      50.000
Es Teh Manis        3 x  5.000      15.000
------------------------------------------------
TOTAL                               65.000
```

## Three entry points

```csharp
Mdpos.Render(template, profile)       // byte[]  ESC/POS bytes
Mdpos.Preview(template, profile)      // string  monospace, for a diff or a console
Mdpos.PreviewHtml(template, profile)  // string  HTML fragment, for showing a person
```

The two previews have different jobs. `Preview` is a developer's diff tool — honest about the
grid and blind to everything else. `PreviewHtml` returns one `<div>` carrying its own scoped
`<style>`, so it renders standalone, cannot be clobbered by host CSS, and can be handed
straight to a WebView.

Its fidelity is resemblance, not pixel accuracy — a browser does not have the printer's ROM
font. That is enough, because the preview is not what enforces fit: layout already wraps
left/centered overflow and rejects right-aligned overflow and an oversized QR, so nothing can
silently run off the paper edge in a document that renders at all.

## Profiles

```csharp
var wide   = PrinterProfile.Epson80mm;                  // 576 dots, 48 columns
var narrow = PrinterProfile.Epson80mm.WithWidthDots(384); // 58mm, 32 columns
var dense  = PrinterProfile.Epson80mm.WithFont(PrinterFont.B); // 64 columns

int columns = wide.ColumnsAt(1);   // 48
int magnified = wide.ColumnsAt(2); // 24 — magnification halves the grid
```

Column counts are always derived from width and font, never hardcoded. The same template
renders on 58mm and 80mm.

## No I/O

This library does not know what a printer is. No sockets, no serial ports, no USB, no
spooler. It produces bytes; delivering them is yours, as are queueing, chunking, retries, job
atomicity and status polling.

That is deliberate rather than minimalist. Printer transport is a platform tarpit, and the
largest target hardware — Sunmi, iMin and Telpo handhelds on Android — exposes nothing but
`sendRAWData(byte[])`. Producing bytes is the only thing that works everywhere.

```csharp
// network / most WiFi printers
using var client = new TcpClient("192.168.1.50", 9100);
client.GetStream().Write(bytes);
```

## Errors

Every template rejection is an `MdposException` whose message carries the 1-based source line:

```
line 3: "1.250.000" overflows right-aligned column 2 (width 6); right-aligned columns never wrap
```

Templates are edited by hand with no compiler in between, so surface that text verbatim to
whoever edits them. It is written for the template author, not for the customer holding the
receipt — log it and show it in your admin UI, not at the till.

## Four things that will bite you

**Whitespace is stripped from both ends of every line and cell.** `Nasi Goreng    | 50.000`
and `Nasi Goreng|50.000` are identical. This is deliberate: templates live in database rows
and text areas that do not preserve trailing spaces, so alignment is *stated* with `:r`, never
implied by padding. For a genuine leading space, escape it — `\ Total`.

**`\` is the only escape rule.** It makes the next character literal: `\|`, `\*`, `\_`, `\{`,
`\\`.

**Column widths are in *current* characters.** Under `{size 2x2}` a width of 20 means 20
double-width characters — 40 base cells. So `{cols 20,10:r,12:r}` totals 42 and is fine at 1x,
but is rejected at 2x, where only 24 columns exist.

**Right-aligned columns never wrap.** Overflow is an error, because a wrapped total prints as
two lines that read as two different numbers.

## Interpolating data

**This library has no data binding and will not grow any.** It renders a finished string.
Building that string — looping over line items, formatting currency and dates — is your
application's job.

Interpolated values become template *source*, so escape them. A product genuinely named
`Nasi Goreng | Spesial` turns a three-cell row into four and the render fails; a name
containing `**` silently toggles bold.

```csharp
static string Escape(string s)
{
    var sb = new StringBuilder(s.Length);
    foreach (char c in s)
    {
        if (c is '\\' or '|' or '*' or '_' or '{') sb.Append('\\');
        sb.Append(c);
    }
    return sb.ToString();
}
```

Two things escaping does not cover: a value that is the entire content of a line and consists
only of dashes becomes a full-width rule (prefix with `\`), and leading or trailing whitespace
is stripped.

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
| `{raw 1D564200}` | Hex passthrough. Spaces allowed. |
| `**text**` | Bold. |
| `__text__` | Underline. |

While a `{cols}` spec is active, lines split on `|` and the cell count must match the spec
exactly. Outside column mode, `|` is ordinary text.

Every document is self-contained: it begins with `ESC @`, ends with a feed and a cut, and
assumes nothing about the printer's prior state. A thermal printer is a stateful interpreter —
leave emphasis on and the *next* receipt prints bold until someone power cycles it.

## Thread safety

Every method is thread-safe and reentrant, and one `PrinterProfile` may be shared by
concurrent callers. The native library holds no global state — rendering is a pure function of
template and profile, and errors travel with the result rather than in a `last_error` slot —
so this wrapper adds no lock.

## Images and logos

There is no image API, and that is a decision rather than a gap. Images go through `{raw}` as
`GS v 0` raster bytes that you build: 1-bit conversion, dithering, padding the width to a
multiple of 8, and the `xL xH yL yH` header are all yours. Justification applies to a `{raw}`
block; nothing else about the payload is examined.

## Format stability

Templates may declare `{v 1}`. The *string* carries the compatibility promise, not the package
version — the engine may be rewritten freely, but a v1 template must render identically in
perpetuity. Record `Mdpos.FormatVersion` alongside stored templates.

## Status

The pipeline is verified on real hardware: an Epson TM-T82X over port 9100, printing
right-aligned prices flush to the margin, a double-width centered header, wrapped product
names, a magnified total row, a scannable payment QR, and a clean cut — with no recompilation
between template edits.

Known limitations:

- **Non-ASCII text is rejected**, not mangled. The CP437 high range is not mapped yet, so
  `café` returns an error rather than printing wrong. Width *measurement* is already Unicode
  display width throughout. QR payloads are exempt — they go out as UTF-8.
- **QR error correction is fixed at M**, and symbol sizing is a deliberate upper bound.
- One built-in profile family (Epson, Font A/B, 58mm and 80mm).
- **Genuine Epson is the target.** Clone printers are not chased. `{raw}` remains the escape
  hatch for a vendor quirk, but that is not a claim of clone support.

Note that "standard ESC/POS" is not a standard. It is Epson's proprietary command set, copied
to varying degrees, with no spec body and no certification. Compatibility claims here are
meant to be falsifiable: print a test template and compare.

## License

Copyright (c) 2026 TERRAKERNEL PTE. LTD.

Licensed under either of Apache License, Version 2.0 or MIT license, at your option.
