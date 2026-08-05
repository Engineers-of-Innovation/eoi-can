#!/usr/bin/env python3
"""Rasterise a TTF into a BDF at a given pixel size, using Pillow's FreeType.

Exists to replace u8g2's `otf2bdf`, which needs libfreetype-dev headers we don't
have. Output is fed to u8g2's bdfconv to produce a .u8g2font blob.

Usage: ttf2bdf.py <font.ttf> <pixel_size> <out.bdf> [glyphs] [weight]
"""
import sys
from PIL import Image, ImageDraw, ImageFont

PAD = 40  # slack around the glyph so nothing clips before we find the ink bbox


def glyph_bitmap(font, ch, ascent, descent):
    """Render one glyph; return (bbx, rows_of_bits) with BDF conventions."""
    w = int(font.getlength(ch)) + 2 * PAD
    h = ascent + descent + 2 * PAD
    img = Image.new("1", (w, h), 0)
    ImageDraw.Draw(img).text((PAD, PAD), ch, fill=1, font=font, anchor="la")

    ink = img.getbbox()
    if ink is None:  # blank glyph, e.g. space
        return (0, 0, 0, 0), []
    x0, y0, x1, y1 = ink
    baseline = PAD + ascent
    bbx = (x1 - x0, y1 - y0, x0 - PAD, baseline - y1)

    px = img.load()
    rows = []
    for y in range(y0, y1):
        rows.append([1 if px[x, y] else 0 for x in range(x0, x1)])
    return bbx, rows


def main():
    ttf, size, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    glyphs = sys.argv[4] if len(sys.argv) > 4 else "-.0123456789: "
    weight = float(sys.argv[5]) if len(sys.argv) > 5 else None

    font = ImageFont.truetype(ttf, size)
    if weight is not None:
        # Variable font: pin the weight axis, leave any others at their default.
        # set_variation_by_axes wants a value per axis, and IBM Plex Sans has two
        # (wght, wdth) -- passing only one silently rasterises a different font
        # than the caller solved the em size against.
        axes = font.get_variation_axes()
        font.set_variation_by_axes([weight] + [a["default"] for a in axes[1:]])

    ascent, descent = font.getmetrics()

    entries = []
    for ch in glyphs:
        bbx, rows = glyph_bitmap(font, ch, ascent, descent)
        entries.append((ch, int(font.getlength(ch)), bbx, rows))

    # Font bounding box must cover every glyph.
    max_w = max(b[0] for _, _, b, _ in entries)
    max_h = max(b[1] for _, _, b, _ in entries)
    min_x = min(b[2] for _, _, b, _ in entries)
    min_y = min(b[3] for _, _, b, _ in entries)

    with open(out, "w") as fh:
        wr = fh.write
        wr("STARTFONT 2.1\n")
        wr("FONT -custom-generated-medium-r-normal--%d-%d-75-75-m-0-iso10646-1\n"
           % (size, size * 10))
        wr("SIZE %d 75 75\n" % size)
        wr("FONTBOUNDINGBOX %d %d %d %d\n" % (max_w, max_h, min_x, min_y))
        wr("STARTPROPERTIES 2\n")
        wr("FONT_ASCENT %d\nFONT_DESCENT %d\n" % (ascent, descent))
        wr("ENDPROPERTIES\n")
        wr("CHARS %d\n" % len(entries))
        for ch, adv, (bw, bh, bx, by), rows in entries:
            wr("STARTCHAR U+%04X\n" % ord(ch))
            wr("ENCODING %d\n" % ord(ch))
            wr("SWIDTH %d 0\n" % int(adv * 1000.0 / size))
            wr("DWIDTH %d 0\n" % adv)
            wr("BBX %d %d %d %d\n" % (bw, bh, bx, by))
            wr("BITMAP\n")
            nbytes = (bw + 7) // 8 if bw else 0
            for row in rows:
                val = 0
                for bit in row:
                    val = (val << 1) | bit
                val <<= (nbytes * 8 - len(row))  # left-align in the byte row
                wr("%0*X\n" % (nbytes * 2, val))
            wr("ENDCHAR\n")
        wr("ENDFONT\n")

    print("%s: %d glyphs at %dpx, ascent=%d descent=%d, bbox %dx%d"
          % (out, len(entries), size, ascent, descent, max_w, max_h))


if __name__ == "__main__":
    main()
