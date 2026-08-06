# Display icons

Warning icons for the 5.79" panel, alongside [`../fonts`](../fonts).

| Icon | Meaning |
| --- | --- |
| `batt` | battery |
| `low` | low charge / range |
| `temp` | over-temperature |
| `throttle` | throttle |

Two forms of each, one of which is compiled in:

| Files | What they are |
| --- | --- |
| `*48.raw` | **what `render.rs` includes** — 48x48 1-bit bitmaps, 288 B each |
| `*.png` | **the master art** — 1-bit at 48x48, what the converter reads |

The panel has no greyscale and no antialiasing, so anything not already pure black
or white gets hard-thresholded by the driver regardless — deciding it here makes
what ships explicit rather than leaving it to the panel.

## Edit the PNGs, at display size

The masters are 48x48 — the size they are drawn at — so the converter does no
scaling and no meaningful thresholding, and `*.png` to `*48.raw` round-trips
byte-identically. Every pixel you set is a pixel on the panel.

They began as 168x168 greyscale stock art, thresholded and reduced. That history is
gone: editing large and reducing loses the detail again on every pass, which is
exactly the wrong loop for 48x48 line art.

If a different size is ever needed, redraw at that size rather than scaling these.

## Embedding

`support/png-to-raw.py` turns the 1-bit `*.png` masters into the `*48.raw` blobs
`render.rs` includes, one 48x48 bitmap per icon at 288 B. Regenerate with:

```sh
support/png-to-raw.py 48 draw-display/assets \
  draw-display/assets/batt.png draw-display/assets/low.png \
  draw-display/assets/temp.png draw-display/assets/throttle.png
```

Two details the converter handles, both of which bit when they were not:

- **A set bit is `BinaryColor::On`, which is this display's *background*** — the
  panel is cleared to `On` and ink is drawn as `Off`. So the converter clears the
  bits where the icon has ink, not the other way round.
- **It scales with an area average and thresholds well above the midpoint.** That
  matters only when the input is larger than the target, as it was when these came
  from 168x168 stock art: a thin line covering part of an output pixel was broken
  into dashes by a midpoint cut, and Lanczos ringing speckled the hard edges. At
  48x48 in, the scale is a no-op and the threshold is exact.

## The speed's numerals

`speed115.raw` is not an icon. It holds the speed's whole numbers — `0`-`9` and
`-` — as 95x115 cells on a common baseline, built by
[`support/ttf-digits-to-raw.py`](../../support/ttf-digits-to-raw.py) from the same
IBM Plex Sans the fonts use:

```sh
support/ttf-digits-to-raw.py /path/to/IBMPlexSans.ttf 500 115 \
  draw-display/assets/speed115.raw
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
