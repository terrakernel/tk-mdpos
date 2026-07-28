# Golden fixtures

Each subdirectory is one case:

```
NNN-short-name/
  input.tmpl      the template
  profile.ron     optional; defaults to Profile::epson_80mm()
  expected.bin    ESC/POS bytes
  expected.txt    monospace preview
```

Both backends snapshot from the same `input.tmpl`, which is what keeps the preview
honest about what the bytes will do.

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

## Cases to write first

These are the four places `INSTRUCTIONS.md` §5 predicts the layout engine will be wrong,
so they are worth having before the engine exists rather than after:

- `001-basic-columns` — the `{cols 20,10:r,12:r}` case from §4.
- `002-double-width-grid` — `{size 2x2}` halving the grid mid-document.
- `003-unicode-wrap` — wrapping where `unicode-width` and `chars().count()` disagree.
- `004-right-align-no-wrap` — overflow in a `:r` column must be an error, not a wrap.

## Publishing

This directory is the seed of the conformance corpus and of the customer-facing
compatibility test — *"print this; if it matches, you're supported."* Structure additions
as if they will be published, even though publishing is out of v0.1 scope.
