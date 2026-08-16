# Changelog

Notable changes to `tk-mdpos`. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is [semver](https://semver.org/), with the caveat that pre-1.0 the minor version
carries breaking changes.

**The template format has its own version, and it is not this one.** `{v 1}` is the public
API and carries the compatibility promise: the engine may be rewritten freely, but a v1
template must render identically in perpetuity. A major bump here does not license a change
in how an existing template prints.

## [0.3.0] — 2026-08-16

First release distributed for Apple platforms and .NET as well as crates.io.

The NuGet package and the CI workflows landed after the crate had already been published to
crates.io the same day. The package deliberately carries the same version: it denotes which
engine ABI the wrapper exposes rather than the wrapper's own history, so `TerraKernel.Mdpos`
0.3.0 wraps `tk-mdpos` 0.3.0. From here the two ship together from one tag.

### Added

- **NuGet package `TerraKernel.Mdpos`**, in `tk-mdpos-dotnet/`. A `net8.0` wrapper over the
  C ABI with no managed dependencies, carrying native binaries for `win-x64` and
  `linux-x64` — that is the whole claim. Apple platforms continue to ship as an XCFramework
  through the Swift package rather than through NuGet, and `android-*` is deliberately
  absent despite Android being the largest target hardware.

  `Mdpos.Render` returns `byte[]`; `Preview` and `PreviewHtml` return `string`. The native
  buffer never escapes the wrapper, so the free-exactly-once rule is structural rather than
  something callers have to honour. Every template rejection is a single `MdposException`
  carrying the source line, because the message is the useful half of a rejection.

  Verified by rendering the entire golden corpus through the wrapper and comparing against
  `expected.bin` byte-for-byte — both against local sources and against the packed `.nupkg`
  consumed from a clean project, which is the only thing that proves the native asset
  actually resolves.

  **Verified on hardware**: a receipt printed from the packaged package on an Epson TM-T82X
  over port 9100, with a double-width centred header, prices flush at column 48, a wrapped
  product name with a hanging indent, a double-width total row also flush right, a scannable
  QR, and a clean partial cut.

- **CI.** `.github/workflows/ci.yml` runs tests, clippy, the `smoke.c` ABI check and a C++
  header compile across Linux, macOS and Windows, plus ASan over `smoke.c`, an MSRV job at
  1.85, and a packaged-crate test. `.github/workflows/release-nuget.yml` builds each RID
  natively on its own runner, packs, verifies by consuming the package on both RIDs, and
  publishes on a `vX.Y.Z` tag.

  `smoke.c` was previously the only check on header drift and ran when someone remembered.
  The Windows leg is verified rather than assumed: the required system libraries are
  `kernel32 ntdll userenv ws2_32 dbghelp`, which is not what was guessed.

- **Swift package.** `Package.swift` at the repository root declares a `binaryTarget`
  pointing at `TkMdpos.xcframework.zip` on this release, so Xcode and SwiftPM can consume
  the C ABI directly. The manifest must sit at the root because a SwiftPM dependency is a
  git URL with no subpath; it shares the repo with the Rust workspace so the checksum always
  refers to an artifact built from the same commit. Verified by building and running a Swift
  consumer against the framework — `import TkMdpos`, render, HTML preview, and free.

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
- **The C ABI's thread-safety guarantee is now documented and tested.** Every entry point
  is thread-safe and reentrant: there is no global, thread-local, or otherwise shared
  mutable state, since rendering is a pure function of `(template, profile)` and errors
  travel in the out-buffer rather than a `last_error` slot. Two consequences hosts may rely
  on: a buffer may be freed on a different thread from the one that produced it, so a
  garbage-collected host can release from a finalizer thread; and one `TkMdposProfile` may
  be shared by concurrent callers. This was already true — what changes is that it is
  stated, so a wrapper author no longer has to assume the conservative thing and serialize
  behind a lock they do not need. Pinned by two tests, verified clean under
  ThreadSanitizer with an instrumented std.
- `mdpos --html` in the CLI.
- `expected.html` in every golden fixture. Existing `expected.bin` and `expected.txt` are
  byte-identical, which is the evidence that nothing in the render path moved.

- **Apple platform artifacts.** `tk-mdpos-ffi/build-xcframework.sh` produces
  `TkMdpos.xcframework` with three slices — universal macOS, iOS device, and a universal iOS
  simulator — plus a hand-written `module.modulemap` beside the header so Swift can
  `import TkMdpos` without a bridging header. `staticlib` rather than `cdylib`, since iOS
  will not load an arbitrary dylib and a static archive needs no runtime lookup. The script
  links `tests/smoke.c` against the finished bundle before reporting success, because a
  framework that assembled is not a framework that links. Not published anywhere yet; the
  distribution route is undecided.

### Changed

- Release artifacts are stripped of debug info. `strip -S` on the static archives took the
  zipped XCFramework from 28 MB to 20 MB — the DWARF is std's, arriving with its precompiled
  objects. Cargo's `strip` profile setting does not reach a staticlib (it is a link-time
  flag, and an ar archive is never linked), so the two mechanisms are separate and both are
  documented where they apply.

### Fixed

- `include/tk_mdpos.h` could not be included from C++, despite its `extern "C"` guards:
  three prototypes named a parameter `template`, which is a keyword there. Renamed to
  `tmpl`. Parameter names in a prototype are documentation only, so this is not an ABI
  change.

- **`.gitattributes` pins the tree to LF.** A Windows clone with the default
  `core.autocrlf=true` checked out the golden fixtures as CRLF and failed every fixture
  compared against LF output, and gave `build-xcframework.sh` a CRLF shebang that macOS
  rejects as a bad interpreter. No engine behaviour was involved — templates themselves
  are unaffected, since the parser splits with `str::lines()` and strips a trailing `\r`.

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
