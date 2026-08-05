# Display fonts

Pre-rasterised u8g2 font blobs for the 5.79" panel, included by
[`../src/render.rs`](../src/render.rs) via `include_bytes!`.

| Blob | Digit height | Advance | Size | Used for |
| --- | --- | --- | --- | --- |
| `plex_speed76_tn` | 76 px | 63 px | 1596 B | speed: whole numbers only |
| `plex_net58_tn` | 58 px | 48 px | 1172 B | net power, the left column's headline |
| `plex_big49_tn` | 49 px | 40 px | 1096 B | state of charge, the speed's dot and tenth, `%` |
| `plex_mid30_tn` | 29 px | 24 px | 567 B | all three times, power in/out, temperatures |
| `plex_small14_tf` | 14 px cap | 12 px | 1842 B | all labels |

All [IBM Plex Sans](https://fonts.google.com/specimen/IBM+Plex+Sans) at **weight
500 (Medium)**, from `ofl/ibmplexsans/IBMPlexSans[wdth,wght].ttf` in
google/fonts. The `wght` axis runs 100–700. OFL-licensed; [`OFL.txt`](OFL.txt)
must ship alongside these blobs.

The value fonts carry ` -.:0123456789`, plus `%` for `plex_big49_tn`, which draws
the state of charge's sign. The label font carries printable ASCII plus U+00B0 for
`°C`. 6273 B total, all in flash — blobs are decoded straight to the draw target,
so they cost no RAM.

A glyph a font is asked to draw but does not have panics through `map_font_err`,
so subsets and call sites have to stay in step. Moving the `%` onto `FONT_BIG` hit
exactly this.

Weight does not affect advances or glyph heights, only stroke thickness, so
changing it needs no layout changes. Every other property does: `render.rs`
hardcodes `SPEED_DIGIT_H`, `SPEED_DIGIT_W`, `SPEED_DOT_W`, `NET_DIGIT_H`,
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

`build-fonts.py` checks both and refuses to write a font that would be corrupt,
so a too-large target fails loudly instead. This is also why the stock
Inconsolata fonts stop at `inb63`.

### Getting past it

The cap is on *advance*, not height, so a narrower face fits more height under
the same limit. Plex Sans has a `wdth` axis (75–100) that has not been used here:

| `wdth` | max digit height at advance ≤ 63 |
| --- | --- |
| 100 (current) | 76 px |
| 90 | 79 px |
| 85 | 80 px |
| 75 | 84 px |

Using it means teaching `ttf2bdf.py` and `build-fonts.py` to pass a width along
with the weight, and accepting condensed digits. The alternatives are patching
`read_unsigned` in a vendored copy of the crate, which removes the limit
entirely, or drawing the speed from `ImageRaw` bitmaps instead of a font.

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
