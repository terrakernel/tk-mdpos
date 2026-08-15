# Changelog

Notable changes to `tk-mdpos`. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is [semver](https://semver.org/), with the caveat that pre-1.0 the minor version
carries breaking changes.

**The template format has its own version, and it is not this one.** `{v 1}` is the public
API and carries the compatibility promise: the engine may be rewritten freely, but a v1
template must render identically in perpetuity. A major bump here does not license a change
in how an existing template prints.

## [Unreleased]

### Added

- **HTML preview backend.** `preview_html()` returns a self-contained fragment — one
  `<div>` carrying its own scoped `<style>` — so it can be embedded in a host page or
  handed straight to a WebView. It draws the three things the monospace backend has to
  discard: emphasis, underline, and magnification at its real printed size. The audience is
  a person approving a receipt layout, not a developer diffing one; `preview()` remains the
  faster loop while editing and is what the fixtures assert the grid against.

  Fidelity is resemblance rather than pixel accuracy, since a browser does not have the
  printer's ROM bitmap font. That is sufficient because the preview does not enforce fit —
  layout already wraps `:l`/`:c` overflow and rejects `:r` overflow and an oversized QR.
  Positions are resolved to whole character cells and expressed in `ch` units, so the grid
  lands correctly whatever monospace font the browser has, and the two previews cannot
  disagree about where a column starts.

  A QR renders as a correctly-sized empty square and `{raw}` as a labelled band. Neither
  gets invented artwork: a drawn QR that is not the symbol the printer will generate invites
  someone to scan it, and a made-up image misreports what `{raw}` contains. The QR payload
  rides on a `data-mdpos-qr` attribute so a host with its own encoder can draw the real
  symbol at the right footprint.
- `tk_mdpos_preview_html` in the C ABI. Unlike byte output the result has no embedded NULs,
  so hosts may treat it as a C string.
- `mdpos --html` in the CLI.
- `expected.html` in every golden fixture. Existing `expected.bin` and `expected.txt` are
  byte-identical, which is the evidence that nothing in the render path moved.

### Fixed

- `include/tk_mdpos.h` could not be included from C++, despite its `extern "C"` guards:
  three prototypes named a parameter `template`, which is a keyword there. Renamed to
  `tmpl`. Parameter names in a prototype are documentation only, so this is not an ABI
  change.

## [0.2.0] — 2026-08-15

First release verified against real hardware. An Epson TM-T82X printed right-aligned prices
flush to the margin, a double-width centred header, wrapped product names, a magnified total
row, and a scannable payment QR — from a hand-edited template with no recompilation.

### Added

- **QR symbols** — `{qr DATA}` and `{qrmod N}` (module size in dots, 1–16, sticky,
  default 6). The printer generates the symbol from `GS ( k`, so there is no encoder, no
  bitmap and no new dependency; only the printed width is computed, so that a symbol too
  wide for the paper is rejected rather than printed clipped and unscannable.
- `Op::Qr { data, module }` in the IR, and a public `qr` module exposing symbol geometry
  (`footprint_dots`, `version_for`, `max_bytes`) for callers that want to check a payload
  before rendering.
- `Error::QrTooWide` and `Error::QrTooLong`.
- Golden fixtures `005-qr-payment` and `006-qr-too-wide`.

### Changed

- **`Error` is now `#[non_exhaustive]`.** *(breaking)* Callers matching on it need a `_`
  arm. This is so future error variants stop being breaking changes — error enums grow with
  every syntax addition, and each one would otherwise force a major version whose entire
  content is a new way of saying no.
- **`Op` gained a variant** and is deliberately **not** `#[non_exhaustive]`. *(breaking)*
  Adding an op should break downstream: anyone matching on `Op` is writing an emitter, and
  an emitter that swallows an unknown op through a `_` arm prints a wrong receipt instead of
  failing to compile.
- Scope is now **genuine Epson ESC/POS**. Clone printers are no longer chased — no
  fallbacks, no per-vendor branches, no runtime probing. `{raw}` remains, but as an escape
  hatch for a caller who needs bytes this crate has no opinion about, not as a claim of
  clone support.

### Fixed

- **`{raw}` blocks are now justified.** `{center}` directly above a `{raw}` emitted nothing,
  so an image sat flush left while the template plainly asked for it centred — and it only
  appeared to work when a printed line above happened to leave `ESC a` set. Found by
  printing the same image four times and comparing positions on paper.

  This changes bytes for affected templates, which is technically a v1 format change. It is
  treated as a fix rather than a break: the only templates whose output moves are the ones
  that were asking to be centred and were not.

### Documented

- **Images** go through `{raw}` as `GS v 0` raster bytes. There is no `{image}` directive
  and this release adds no code for one — a bitmap cannot live in a template string meant to
  sit in a database row, and converting a logo to 1-bit is image processing with an aesthetic
  judgement in the middle of it. The caller pre-processes; this crate passes through. What
  that costs is stated plainly: no width validation, no preview, and no chance of catching an
  `xL`/`xH` header mismatch, which is the classic ESC/POS image bug.

### Known limitations

- **Non-ASCII *text* is rejected, not mangled.** The CP437 high range (0x80–0xFF) is not
  mapped, so `café` returns an error rather than printing wrong. Width *measurement* is
  `unicode-width` throughout. QR payloads are exempt — they are emitted as UTF-8 and bypass
  the code page entirely.
- **QR error correction is fixed at M**, and symbol sizing is a deliberate upper bound
  (capacity is computed for byte mode, the most expensive encoding), so a template within a
  few dots of the paper edge may be rejected even though it would have fit. The error is
  always in that direction.
- One built-in profile: 80mm, Font A, Epson. No profile registry or TOML loading.

## [0.1.0] — 2026-07-28

Initial release. Parser, layout engine, ESC/POS and monospace-preview backends, `Error` with
source line numbers, and four golden fixtures covering the cases where the layout engine was
most likely to be wrong.

Published without having printed anything.

`{v 1}` templates from this release render identically under 0.2.0, with one deliberate
exception: a `{center}` or `{right}` immediately above a `{raw}` block now emits the `ESC a`
it always should have. See *Fixed* above.

[0.2.0]: https://github.com/terrakernel/tk-mdpos/releases/tag/v0.2.0
[0.1.0]: https://github.com/terrakernel/tk-mdpos/releases/tag/v0.1.0
