# Display fonts

Pre-rasterised u8g2 font blobs for the 5.79" panel, included by
[`../src/render/mod.rs`](../src/render/mod.rs) via `include_bytes!`.

| Blob | Digit height | Advance | Size | Used for |
| --- | --- | --- | --- | --- |
| `plex_net58_tn` | 58 px | 48 px | 1172 B | net power, the left column's headline |
| `plex_big49_tn` | 49 px | 40 px | 1096 B | state of charge, the speed's dot and tenth, `%` |
| `plex_mid30_tn` | 29 px | 24 px | 567 B | all three times, power in/out, temperatures |
| `plex_small14_tf` | 14 px cap | 12 px | 1915 B | the dashboard's labels |
| `plex_small12_tf` | 12 px cap | 10 px | 1599 B | the foiling screen's rows |
| `plex_semi12_tf` | 12 px cap | 10 px | 1619 B | the foiling screen's table headings |

All [IBM Plex Sans](https://fonts.google.com/specimen/IBM+Plex+Sans) at **weight
500 (Medium)** except `plex_semi12_tf`, which is **600 (SemiBold)** so the foiling
screen's headings read as headings, from
`ofl/ibmplexsans/IBMPlexSans[wdth,wght].ttf` in google/fonts — exactly:

```
https://raw.githubusercontent.com/google/fonts/main/ofl/ibmplexsans/IBMPlexSans%5Bwdth%2Cwght%5D.ttf
```

The `wght` axis runs 100–700. Note the file name lists its axes alphabetically
but `fvar` orders them `wght` then `wdth`, which is what `ttf2bdf.py` assumes
when it pins the weight and leaves the rest at their defaults. OFL-licensed; [`OFL.txt`](OFL.txt)
must ship alongside these blobs.

The value fonts carry ` -.:0123456789`, plus `%` for `plex_big49_tn`, which draws
the state of charge's sign. The label font carries printable ASCII, U+00B0 for
`°C`, and U+2191/U+2193 for the foiling screen, which marks a diverged up/down
parameter pair as `5.0↑ 8.0↓` and collapses a symmetric one to a single number.
An arrow is 14 px wide at 12 px cap — wider than a two-digit number — which is
what sizes that screen's pitch column.

The foiling screen runs two points smaller than the dashboard because it puts four
tables abreast. A separate blob rather than shrinking the shared one: every
constant in `render/dashboard.rs` derives from a 14 px cap, and that layout is
tuned. 7968 B total, all in flash — blobs are decoded straight to the draw target,
so they cost no RAM.

`plex_semi12_tf` is the same 12 px cap as `plex_small12_tf` and differs only in
weight, so the headings sit on the same row grid as the rows beneath them. It stops
at SemiBold rather than Bold because at this cap a bold `r` arm touches the
following `n` and the Turn table's heading reads as "Tum"; the gap survives at 600
and is gone by 650. Nothing can test that — both glyphs are drawn exactly where the
font says — so it is checked by eye at 6x when the weight changes.

A glyph a font is asked to draw but does not have panics through `map_font_err`,
so subsets and call sites have to stay in step. Moving the `%` onto `FONT_BIG` hit
exactly this.

Weight leaves glyph *heights* alone but does widen advances: building the value
fonts at 600 pushes `plex_big49_tn`'s advance to 64 px, which needs 8 bits and trips
the decoder bug below. Anything measured from a font has to be measured from the one
that draws it -- `headings_fit_beside_each_other` measures the headings against
`FONT_HEADING`, not against `FONT_TINY`. Every other property is hardcoded: `render.rs`
hardcodes `SPEED_DOT_W`, `NET_DIGIT_H`,
`NET_DIGIT_W`, `NET_MINUS_W`, `BIG_DIGIT_H`, `BIG_DIGIT_W`, `MID_DIGIT_H`,
`MID_DIGIT_W`, `MID_COLON_W`, `SMALL_CAP_H`, `SMALL_DIGIT_W`, `SMALL_DEG_C_W` and
the label widths so
the layout is const arithmetic. `font_metrics_match_the_layout` fails if a
regenerated blob no longer matches.

## Why these choices

- **Proportional, not monospace.** Plex Sans has tabular figures — all ten digits
  share one advance — so values don't jitter as they change without forcing a
  monospace build. Monospace mode pads `-`, `.` and `:` out to a full digit cell,
  which reads as `- 2019` and `17: 42: 23` in a proportional face.
  `font_metrics_match_the_layout` checks every value font's digits are equal
  width; without that the right-aligned values would shuffle.
- **Subset per font.** The value fonts carry only the glyphs `fmt_f32`,
  `fmt_hms` and `split_speed` can emit. Subsetting is why these are *smaller*
  than the stock Inconsolata + Helvetica set they replaced (8085 B) despite Plex
  Sans being a wider face.
- **Sizes are per-blob.** u8g2 fonts are bitmaps with no scaling, so every
  distinct size is its own blob. Unused fonts cost nothing, but each one you
  reference costs its full size in flash.

Run `cargo test -p draw-display` after regenerating. Besides the metric checks,
`widest_values_fit_their_cells`, `speed_pieces_land_inside_the_centre_column`,
`rows_fit_the_top_band` and `times_fit_the_bottom_row` bound the fonts against
the layout — `render_aligned` clips silently rather than erroring, so a font that
outgrows its cell produces no warning at all.

## 63 px is the hard ceiling on advance width

**Do not raise a value font past a 63 px advance.** The speed font already sits
exactly at that limit: 76 px digits, em 106, advance 63. At 77 px (em 107) the
advance becomes 64, and `bdfconv` needs 8 bits for the advance field —
`u8g2-fonts` 0.7.2 cannot decode 8-bit fields:
`read_unsigned` does `value2.overflowing_shl(8 - bit_start)`, and `shl(8)` on a
`u8` is a no-op, so the next byte is OR'd in unshifted. The font compiles,
`bdfconv` exits 0, and the result renders with wrong advances and non-tabular
digits — at 79 px digits, `0` reported an advance of 89 and `1` reported 83.

`bdfconv` has a second silent trap: it does not verify that the widest glyph fits
the bits it allocated, and gave 6 bits (max 63) to a 64 px-wide glyph.

Check that against the **per-glyph** maximum, not `bdfconv`'s `CalculateMaxBBX`.
That reports the *union* of every glyph box — `max(y_off + h) - min(y_off)` — which
for a text font is always a pixel or two taller than the tallest single glyph,
because whatever reaches highest is not what descends deepest. A u8g2 glyph stores
its own width and height in those fields, so the union is the wrong yardstick:
comparing against it rejects sound fonts. It is what made a 12 px cap look
impossible — `bdfconv` had correctly given `bbx.h` five bits for a 16 px glyph, and
the check compared that against a 17 px union.

`build-fonts.py` checks both and refuses to write a font that would be corrupt,
so a too-large target fails loudly instead.

Only 22 of the 1994 bundled fonts carry any field wider than 7 bits, so the limit
is rarely hit in practice — the stock Inconsolata set is nowhere near it
(`inb63_mn` is 82 px tall with an advance of only 51).

### The speed went around it

The speed's whole numerals are not a font at all. They are bitmaps, rasterised by
[`support/ttf-digits-to-raw.py`](../../support/ttf-digits-to-raw.py) into
`../assets/speed115.raw`, which sidesteps the u8g2 bit fields entirely: 115 px
digits where a font caps out at 76. See [`../assets/README.md`](../assets/README.md).

The routes not taken, if a *font* ever needs to be bigger:

- **`logisoso*_tn`**, already bundled, reaches 92 px at an advance of 59 — taller
  *and* narrower than Plex Sans manages under the limit, verified tabular and
  rendering correctly for every glyph. It is a different typeface, which is the
  only reason it is not used.
- **Plex Sans has a `wdth` axis (75–100)** that has not been used here. Narrower
  fits more height under the same advance cap: 79 px at `wdth` 90, 84 px at 75.
  It means teaching `ttf2bdf.py` and `build-fonts.py` to pass a width.
- **Patching `read_unsigned`** in a vendored copy of the crate removes the limit
  outright.

## Regenerating

Needs Pillow with FreeType (`pip install pillow`) and u8g2's `bdfconv`, which is
not packaged anywhere — build it from the upstream repo:

```sh
git clone --depth 1 --filter=blob:none --sparse https://github.com/olikraus/u8g2.git
cd u8g2 && git sparse-checkout set tools/font && make -C tools/font/bdfconv
```

Then, from the repo root:

```sh
BDFCONV=/path/to/u8g2/tools/font/bdfconv/bdfconv \
  support/build-fonts.py /path/to/IBMPlexSans.ttf 500 draw-display/fonts
```

**Check the regeneration reproduced the old blobs.** The three digit fonts have a
fixed glyph set, so any change to them means the face, weight or pipeline moved,
not the subset:

```sh
cargo test -p draw-display          # font_metrics_match_their_consts pins every advance
git status draw-display/fonts/       # only the blob whose subset you changed may differ
```

`font_metrics_match_their_consts` fails if an advance drifts, and the golden
renders in [`../tests/golden.rs`](../tests/golden.rs) fail if a single pixel
moves. Adding U+2191/U+2193 left `plex_net58_tn`, `plex_big49_tn` and
`plex_mid30_tn` byte-identical and both dashboard hashes unchanged, which is what
a subset-only change should look like.

**A missing glyph is not an error.** `bdfconv` drops a character the TTF lacks
without a word, and `u8g2-fonts` then panics *inside its own glyph search* when
asked to draw it — before `map_font_err` can turn it into a `Result`. So coverage
has to be tested, not handled: see `every_glyph_this_screen_draws_exists` in
[`../src/render/foiling.rs`](../src/render/foiling.rs).

The pipeline is `TTF → BDF → C array → raw blob`:

1. [`support/ttf2bdf.py`](../../support/ttf2bdf.py) rasterises the TTF at a given
   em size via Pillow's FreeType. This replaces u8g2's `otf2bdf`, which needs
   `libfreetype-dev` headers; Pillow bundles FreeType already.
2. `bdfconv` compresses the BDF into a u8g2 font, emitted as a C array.
3. [`support/c2u8g2font.py`](../../support/c2u8g2font.py) decodes that C string
   literal into the raw bytes this directory holds.

`build-fonts.py` drives all three and solves the em size for each target glyph
height, because u8g2 names fonts by glyph height while TTF sizes are em sizes —
Plex Sans needs em 67 to produce 49 px digits.

## Changing the face or weight

Weight is the second argument to `build-fonts.py`; nothing else needs touching.

Point it at a different TTF to change face. Two things to check on any candidate:

- **Tabular figures.** Without them digits change width and values visibly
  shuffle on every refresh. Verify with
  `len({font.getlength(d) for d in "0123456789"}) == 1`, or build monospace
  (bdfconv `-b 2`) and accept the punctuation padding.
- **Weight at 1 bit.** The panel has no antialiasing, so every glyph is hard
  thresholded. Medium holds up at these sizes because the digits are large;
  anything lighter starts to break up, and the 14 px label font is where it shows
  first.
