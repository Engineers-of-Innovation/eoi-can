# Display icons

Warning icons for the 5.79" panel, alongside [`../fonts`](../fonts).

| Icon | Meaning |
| --- | --- |
| `batt` | battery |
| `low` | low charge / range |
| `temp` | over-temperature |
| `throttle` | throttle |

Three forms of each, only one of which is compiled in:

| Files | What they are |
| --- | --- |
| `*48.raw` | **what `render.rs` includes** — 48x48 1-bit bitmaps, 288 B each |
| `*.png` | the same art 1-bit at full 168x168, as a human-viewable reference |
| `source/*.png` | the greyscale originals, the input the converter scales from |

The panel has no greyscale and no antialiasing, so anything not already pure black
or white gets hard-thresholded by the driver regardless — deciding it here makes
what ships explicit rather than leaving it to the panel.

## Why the greyscale originals are kept

They are the better input for the display-sized bitmaps. The art is 168x168 and is
drawn at 48x48, and reducing *1-bit* art by 3.5x aliases badly: thin strokes drop
out and edges break up. Scaling the greyscale source first and thresholding at the
target size keeps the shapes. `source/` is ~20 kB and is compiled into nothing.

## Embedding

`support/png-to-raw.py` turns `source/*.png` into the `*48.raw` blobs `render.rs`
includes, one 48x48 bitmap per icon at 288 B. Regenerate with:

```sh
support/png-to-raw.py 48 draw-display/assets draw-display/assets/source/*.png
```

Two details the converter handles, both of which bit when they were not:

- **A set bit is `BinaryColor::On`, which is this display's *background*** — the
  panel is cleared to `On` and ink is drawn as `Off`. So the converter clears the
  bits where the icon has ink, not the other way round.
- **It scales with an area average, not Lanczos, and thresholds well above the
  midpoint.** The battery icon's bottom border is a thin line in the 168px source
  covering only part of a 48px output pixel; a midpoint cut broke it into dashes,
  and Lanczos ringing added speckle around the hard edges. The stock watermark
  (grey 244-254) is flattened to white first so it cannot tint the average of the
  pixels it overlaps.

## The speed's numerals

`speed105.raw` is not an icon. It holds the speed's whole numbers — `0`-`9` and
`-` — as 86x105 cells on a common baseline, built by
[`support/ttf-digits-to-raw.py`](../../support/ttf-digits-to-raw.py) from the same
IBM Plex Sans the fonts use:

```sh
support/ttf-digits-to-raw.py /path/to/IBMPlexSans.ttf 500 105 \
  draw-display/assets/speed105.raw
```

They are bitmaps because u8g2-fonts caps a font at a 63 px advance, which is 76 px
digits for this face — see [`../fonts/README.md`](../fonts/README.md). Uniform
cells mean the `--` placeholder is exactly as wide as a real `14`, so the block
never shifts. `render.rs` positions the dot and tenth from the cell's side
bearings, so `SPEED_DOT_GAP` is ink-to-ink air rather than box-to-box.

## When each icon shows

`icon_conditions` in `render.rs`, checked by
`icon_conditions_follow_their_thresholds`:

| Icon | Raised when |
| --- | --- |
| `batt` | battery state, charge state or discharge state is abnormal |
| `low` | state of charge below 15% |
| `temp` | motor above 50 °C, driver above 70 °C, hottest MPPT above 80 °C, or hottest battery thermistor above 45 °C |
| `throttle` | the throttle reports any error |

These replaced the inverted `BAT!`/`THR!` text badge, which is gone. Positions are
fixed, so an inactive icon leaves its slot empty rather than the others shifting —
a warning always appears in the same place. A stale value raises nothing:
`DisplayValue::get` returns `None` once it times out, so no warning latches.
