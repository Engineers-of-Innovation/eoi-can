#!/usr/bin/env python3
"""Build the draw-display font blobs from a variable TTF.

Solves the TTF em size for each target glyph height (u8g2 names fonts by glyph
height, not em size), rasterises to BDF, compiles with u8g2's bdfconv, and
writes the raw .u8g2font blobs the Font impls include_bytes!.

Needs Pillow (with FreeType) and u8g2's bdfconv. See draw-display/fonts/README.md
for how to get bdfconv; point BDFCONV at it if it is not on PATH.

Usage: build-fonts.py <font.ttf> <weight> <outdir>
   eg: support/build-fonts.py ~/IBMPlexSans.ttf 500 draw-display/fonts
"""
import os
import re
import shutil
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
BDFCONV = os.environ.get("BDFCONV") or shutil.which("bdfconv")
TTF2BDF = os.path.join(HERE, "ttf2bdf.py")
C2BIN = os.path.join(HERE, "c2u8g2font.py")

DIGITS = " -.:0123456789"
DIGIT_MAP = "32,45,46,48-58"
# FONT_BIG additionally draws the state of charge's percent sign. A missing glyph
# panics through map_font_err, so every character a font is asked to draw must be
# in its subset.
DIGITS_PCT = DIGITS + "%"
DIGIT_PCT_MAP = "32,37,45,46,48-58"
# Printable ASCII plus U+00B0 DEGREE SIGN, which "°C" needs.
ASCII = "".join(chr(c) for c in range(32, 127)) + "°"
ASCII_MAP = "32-126,176"

# (blob name, measured glyph, target height in px, glyph set, bdfconv map, build mode)
#   Everything uses build mode 0 (proportional). Plex Sans has tabular figures --
#   all ten digits share one advance -- so values don't jitter without forcing
#   monospace, and '-', '.' and ':' keep their natural narrow widths. Monospace
#   mode (2), which the stock Inconsolata _mn fonts use, gives punctuation a full
#   digit cell; in a proportional face that reads as "- 2019" and "17: 42: 23".
SPECS = [
    # Net power: between the speed and the plain values, and wide enough for
    # "-2000" in the left column.
    ("plex_net58_tn",   "0", 58, DIGITS, DIGIT_MAP, 0),
    # The right column's three values.
    ("plex_big49_tn",   "0", 49, DIGITS_PCT, DIGIT_PCT_MAP, 0),
    ("plex_mid30_tn",   "0", 30, DIGITS, DIGIT_MAP, 0),
    ("plex_small14_tf", "T", 14, ASCII,  ASCII_MAP, 0),
]


def glyph_height(ttf, weight, em, ch):
    f = ImageFont.truetype(ttf, em)
    f.set_variation_by_axes([weight, 100])  # [wght, wdth]
    img = Image.new("1", (em * 3 + 20, em * 3 + 20), 0)
    ImageDraw.Draw(img).text((em, em), ch, fill=1, font=f, anchor="la")
    box = img.getbbox()
    return 0 if box is None else box[3] - box[1]


def solve_em(ttf, weight, ch, target):
    """Smallest em size whose glyph height is >= target, else the closest under."""
    best, best_err = None, None
    for em in range(6, 240):
        h = glyph_height(ttf, weight, em, ch)
        err = abs(h - target)
        if best_err is None or err < best_err:
            best, best_err = em, err
        if h > target + 4:
            break
    return best, glyph_height(ttf, weight, best, ch)


# Widest bit field u8g2-fonts 0.7.2 can decode. Its read_unsigned does
# `value2.overflowing_shl(8 - bit_start)`, and shl(8) on a u8 is a no-op, so an
# 8-bit field that straddles a byte boundary ORs in the next byte unshifted and
# returns garbage. bdfconv happily emits 8-bit fields, exits 0, and the font
# renders with wrong advances and non-tabular digits.
MAX_FIELD_BITS = 7


def check_bitfields(name, verbose_output):
    """Fail on a font bdfconv encoded in a way that cannot be decoded correctly.

    Two separate traps, both silent -- bdfconv exits 0 either way:

    1. It does not verify that the widest glyph fits the bits it allocated. At
       79px Plex Sans digits it gave 6 bits (max 63) to a 64px-wide glyph.
    2. Any field wider than MAX_FIELD_BITS hits the crate's decode bug above.
       Plex Sans crosses this at em 107, where an advance of 64 forces 8 bits.

    This is also why the stock Inconsolata fonts stop at inb63.
    """
    maxbbx = re.search(r"CalculateMaxBBX: x=\S+ y=\S+ w=(\d+), h=(\d+)", verbose_output)
    fields = re.search(r"bf_CalculateMaxBitFieldSize: bbx\.x=(\d+), bbx\.y=(\d+), "
                       r"bbx\.w=(\d+), bbx\.h=(\d+), dwidth=(\d+)", verbose_output)
    dwidth = re.search(r"bf_CalculateMinMaxDWidth: dx_min=(-?\d+), dx_max=(-?\d+)",
                       verbose_output)
    if not maxbbx or not fields or not dwidth:
        sys.exit("%s: could not parse bdfconv's bit field report; refusing to "
                 "ship an unverified font" % name)

    bits = dict(zip(("x", "y", "w", "h", "dwidth"), (int(g) for g in fields.groups())))

    for field, nbits in bits.items():
        if nbits > MAX_FIELD_BITS:
            sys.exit("%s: bdfconv used %d bits for '%s', more than the %d "
                     "u8g2-fonts can decode -- the font would render with wrong "
                     "advances. Reduce the target size."
                     % (name, nbits, field, MAX_FIELD_BITS))

    for what, value, nbits in (("width", int(maxbbx.group(1)), bits["w"]),
                               ("height", int(maxbbx.group(2)), bits["h"]),
                               ("advance", int(dwidth.group(2)), bits["dwidth"])):
        limit = (1 << nbits) - 1
        if value > limit:
            sys.exit("%s: glyph %s %d exceeds the %d bits bdfconv allocated "
                     "(max %d). The font would be silently corrupt -- reduce the "
                     "target size." % (name, what, value, nbits, limit))


def main():
    if not BDFCONV or not os.path.exists(BDFCONV):
        sys.exit("bdfconv not found; set BDFCONV=/path/to/bdfconv "
                 "(see draw-display/fonts/README.md)")

    ttf, weight, outdir = sys.argv[1], float(sys.argv[2]), sys.argv[3]
    os.makedirs(outdir, exist_ok=True)
    total = 0

    for name, probe, target, glyphs, cmap, mode in SPECS:
        em, got = solve_em(ttf, weight, probe, target)
        bdf = os.path.join(outdir, name + ".bdf")
        cfile = os.path.join(outdir, name + ".c")
        blob = os.path.join(outdir, name + ".u8g2font")

        subprocess.run([sys.executable, TTF2BDF, ttf, str(em), bdf, glyphs, str(weight)],
                       check=True, stdout=subprocess.DEVNULL)
        # -v so check_bitfields can see the packing decisions.
        out = subprocess.run([BDFCONV, "-v", "-f", "1", "-b", str(mode), "-m", cmap,
                              bdf, "-o", cfile, "-n", "u8g2_font_" + name],
                             check=True, capture_output=True, text=True).stdout
        check_bitfields(name, out)
        subprocess.run([sys.executable, C2BIN, cfile, blob],
                       check=True, stdout=subprocess.DEVNULL)

        size = os.path.getsize(blob)
        total += size
        print("%-16s em=%-3d %s height %2d px (target %2d)  %5d B"
              % (name, em, probe, got, target, size))
        os.remove(bdf)
        os.remove(cfile)

    print("total: %d B" % total)


if __name__ == "__main__":
    main()
