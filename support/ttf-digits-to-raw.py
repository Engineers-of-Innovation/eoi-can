#!/usr/bin/env python3
"""Rasterise digits from a TTF into fixed-cell 1-bit bitmaps.

For the one value that wants to be bigger than a u8g2 font can be. u8g2-fonts
0.7.2 cannot decode a bit field wider than 7, which caps a font at a 63px advance
-- 76px digits for IBM Plex Sans. Drawing those glyphs as bitmaps sidesteps the
font machinery entirely, at the cost of flash and a fixed size.

Every glyph gets the same cell: the digit advance wide, the digit height tall,
each placed on a common baseline. Digits are tabular so they fill their cell, and
'-' sits where it belongs within it. Uniform cells mean the placeholder "--" is
exactly as wide as a real "14", so nothing shifts.

Output is one blob, glyphs concatenated in GLYPHS order, MSB-first with each row
padded to a whole byte -- the layout `ImageRaw::<BinaryColor>` expects. A set bit
is `BinaryColor::On`, this display's background, so ink leaves the bit clear.

Usage: ttf-digits-to-raw.py <font.ttf> <weight> <digit-height> <out.raw>
   eg: support/ttf-digits-to-raw.py ~/IBMPlexSans.ttf 500 86 \\
           draw-display/assets/speed86.raw
"""
import sys

from PIL import Image, ImageDraw, ImageFont

# Order matters: render.rs indexes into the blob by this.
GLYPHS = "0123456789-"
PAD = 60


def solve_em(ttf, weight, target_height):
    """Em size whose digit ink is `target_height` tall."""
    best, best_err = None, None
    for em in range(10, 400):
        if measure(ttf, weight, em)[0] > target_height + 6:
            break
        err = abs(measure(ttf, weight, em)[0] - target_height)
        if best_err is None or err < best_err:
            best, best_err = em, err
    return best


def font_at(ttf, weight, em):
    f = ImageFont.truetype(ttf, em)
    axes = f.get_variation_axes()
    f.set_variation_by_axes([weight] + [a["default"] for a in axes[1:]])
    return f


def measure(ttf, weight, em):
    """(digit ink height, digit advance) at this em size."""
    f = font_at(ttf, weight, em)
    img = Image.new("1", (em * 3, em * 3), 0)
    ImageDraw.Draw(img).text((em, em), "0", fill=1, font=f, anchor="la")
    box = img.getbbox()
    return (box[3] - box[1], int(f.getlength("0")))


def main():
    ttf, weight, height, out = sys.argv[1], float(sys.argv[2]), int(sys.argv[3]), sys.argv[4]

    em = solve_em(ttf, weight, height)
    digit_h, cell_w = measure(ttf, weight, em)
    f = font_at(ttf, weight, em)
    ascent, _ = f.getmetrics()
    row_bytes = (cell_w + 7) // 8

    # Where a digit's ink starts, relative to the baseline. Every glyph is cropped
    # against this so they share one baseline.
    probe = Image.new("1", (em * 4, em * 4), 0)
    ImageDraw.Draw(probe).text((PAD, PAD), "0", fill=1, font=f, anchor="la")
    top = probe.getbbox()[1]

    blob = bytearray()
    for ch in GLYPHS:
        canvas = Image.new("1", (em * 4, em * 4), 0)
        ImageDraw.Draw(canvas).text((PAD, PAD), ch, fill=1, font=f, anchor="la")
        # Same crop window for every glyph: the pen position across, the digit ink
        # band down. Anything outside is padding, which is what we want.
        cell = canvas.crop((PAD, top, PAD + cell_w, top + digit_h))

        px = cell.load()
        for y in range(digit_h):
            row = bytearray(row_bytes)
            for x in range(cell_w):
                if not px[x, y]:
                    # Background: set the bit. Ink leaves it clear.
                    row[x // 8] |= 0x80 >> (x % 8)
            blob += row

    with open(out, "wb") as fh:
        fh.write(blob)

    print("%s: %d glyphs %r, cell %dx%d (%d B), em %d, total %d B"
          % (out, len(GLYPHS), GLYPHS, cell_w, digit_h,
             row_bytes * digit_h, em, len(blob)))
    print("  render.rs needs: SPEED_DIGIT_W = %d, SPEED_DIGIT_H = %d"
          % (cell_w, digit_h))


if __name__ == "__main__":
    main()
