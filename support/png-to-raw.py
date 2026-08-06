#!/usr/bin/env python3
"""Convert icon PNGs into raw 1-bit bitmaps for `embedded_graphics::image::ImageRaw`.

Scales down to the target size and thresholds: the icons are 168x168 and the
display draws them at 48x48. The threshold is deliberately forgiving, because
reducing by 3.5x leaves a thin stroke covering only a fraction of an output pixel.

Bit order is MSB-first with each row padded to a whole byte, which is what
`ImageRaw::<BinaryColor>` expects. A set bit is `BinaryColor::On`, and this
project clears the display to `On` and draws ink as `Off` -- so a bit is **0**
where the icon is black and 1 for background.

Usage: png-to-raw.py <size> <out-dir> <master.png>...
   eg: support/png-to-raw.py 48 draw-display/assets \\
           draw-display/assets/batt.png draw-display/assets/low.png \\
           draw-display/assets/temp.png draw-display/assets/throttle.png
"""
import os
import sys

from PIL import Image

# A pixel darker than this in the scaled-down image becomes ink. Well above the
# midpoint on purpose: a 1px border in a 168px source covers only a fraction of a
# 48px output pixel, so area-averaging leaves it mid-grey. A midpoint cut broke the
# battery icon's bottom border into dashes.
INK_THRESHOLD = 200

# The stock icons carry a near-white spiral watermark (grey 244-254). Flattening it
# to pure white before scaling keeps it from tinting the average of pixels it
# overlaps, which is how it reached the bottom border of the battery.
WATERMARK_FLOOR = 240


def to_raw(path, size):
    src = Image.open(path).convert("L")
    src = src.point(lambda p: 255 if p >= WATERMARK_FLOOR else p)
    # BOX is a plain area average. LANCZOS rings around the hard black/white edges
    # these icons are made of, which shows up as speckle after thresholding.
    scaled = src.resize((size, size), Image.BOX)

    row_bytes = (size + 7) // 8
    out = bytearray()
    ink = 0
    px = scaled.load()
    for y in range(size):
        row = bytearray(row_bytes)
        for x in range(size):
            if px[x, y] < INK_THRESHOLD:
                ink += 1
            else:
                # Background: set the bit. Ink leaves it clear.
                row[x // 8] |= 0x80 >> (x % 8)
        out += row
    return bytes(out), ink


def main():
    size = int(sys.argv[1])
    out_dir = sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)

    for path in sys.argv[3:]:
        name = os.path.splitext(os.path.basename(path))[0]
        data, ink = to_raw(path, size)
        out = os.path.join(out_dir, "%s%d.raw" % (name, size))
        with open(out, "wb") as fh:
            fh.write(data)
        print("%-18s %dx%d, %d B, %d ink px (%.1f%%)"
              % (os.path.basename(out), size, size, len(data), ink,
                 100.0 * ink / (size * size)))


if __name__ == "__main__":
    main()
