# Golden fixtures

Each subdirectory is one case:

```
NNN-short-name/
  input.tmpl      the template
  profile.ron     optional; defaults to Profile::epson_80mm()
  expected.bin    ESC/POS bytes
  expected.txt    monospace preview
  expected.html   HTML preview
  expected.err    instead of the three above: the rejection this template must produce
```

A fixture with `expected.err` asserts that the template is *refused*, matching `Error`'s
`Display` output. Refusing a template is a feature — a wrapped total is worse than a
failed render — so the corpus has to pin the refusals too.

All three backends snapshot from the same `input.tmpl`, which is what keeps the previews
honest about what the bytes will do.

`expected.html` pins positions and sizes, not appearance. Whether the HTML preview *looks*
right is only answerable by opening it next to printed paper; what the fixture catches is a
cell moving, or the two previews drifting apart.

Run with `cargo test --test golden`. Regenerate deliberately:

```
UPDATE_GOLDEN=1 cargo test --test golden
```

Then read the diff. A changed `.bin` is a v1 compatibility break until proven otherwise —
deployed templates must render identically in perpetuity.

`profile.ron` looks like:

```ron
(
    dialect: Epson,
    width_dots: 576,
    font: A,
    code_page: Cp437,
    supports_partial_cut: true,
)
```

## Current cases

These cover the four places `INSTRUCTIONS.md` §5 predicts the layout engine will be wrong:

- `001-basic-columns` — the `{cols 20,10:r,12:r}` receipt from §4.
- `002-double-width-grid` — `{size 2x2}` halving the grid mid-document, including a
  magnified column row whose positions must scale with it.
- `003-long-name-wrap` — long product names wrapping, with a hanging indent on a column
  that is *not* the first one, so continuation lines have somewhere wrong to go.
- `004-right-align-no-wrap` — overflow in a `:r` column is an error, not a wrap.

Added with QR support in v0.2:

- `005-qr-payment` — a receipt ending in a centered payment code. The payload is a
  QRIS-shaped string with obviously fake identifiers, at 206 bytes, which is
  representative of the real thing: version 10, 65 modules including the quiet zone, 390
  dots at the default module size. The preview draws the symbol at its true printed
  width, so this fixture pins the geometry as well as the bytes.
- `006-qr-too-wide` — the same payload at `{qrmod 9}`, needing 585 dots against 576. A
  clipped payment code is unscannable, so this is a refusal for the same reason
  `004` is.

## Still missing

A unicode fixture, which is the one §5.3 asks for. It is blocked on the CP437 high range
(0x80..=0xFF) in `emit::escpos::encode_text`: until that table exists, any non-ASCII
character is rejected, so no `expected.bin` can be produced. Width *measurement* is
already `unicode-width` throughout and covered by unit tests in `layout` and
`emit::preview`. Add the fixture with the encoder table, not before.

## Publishing

This directory is the seed of the conformance corpus and of the customer-facing
compatibility test — *"print this; if it matches, you're supported."* Structure additions
as if they will be published, even though publishing is out of v0.1 scope.
