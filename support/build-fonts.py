#!/usr/bin/env python3
"""Build the draw-display font blobs from a variable TTF.

Solves the TTF em size for each target glyph height (u8g2 names fonts by glyph
height, not em size), rasterises to BDF, compiles with u8g2's bdfconv, and
writes the raw .u8g2font blobs the Font impls include_bytes!.

Needs Pillow (with FreeType) and u8g2's bdfconv. See draw-display/fonts/README.md
for how to get bdfconv; point BDFCONV at it if it is not on PATH.

Usage: build-fonts.py <font.ttf> <weight> <outdir> [blob ...]
   eg: support/build-fonts.py ~/IBMPlexSans.ttf 500 draw-display/fonts
       support/build-fonts.py ~/IBMPlexSans.ttf 500 /tmp/f plex_semi12_tf

Naming blobs builds only those, which is how one font is re-tuned without
rebuilding -- or having to satisfy the size checks of -- the other four.
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
# Printable ASCII, plus U+00B0 DEGREE SIGN for "°C" and U+2191/U+2193 ARROWS for
# the foiling screen, which marks an asymmetric up/down parameter pair as
# "5.0↑ 8.0↓" and collapses a symmetric one to a single number. U+00B5 MICRO SIGN
# is for that screen's units column ("µs", the rear-foil jog PWM).
#
# Verify after regenerating: bdfconv drops a glyph the TTF does not have without
# complaining, and a missing glyph only shows up as a `map_font_err` panic at
# runtime. `fonts/README.md` has the check.
ASCII = "".join(chr(c) for c in range(32, 127)) + "°↑↓µ"
ASCII_MAP = "32-126,176,8593,8595,181"

# (blob name, measured glyph, target height in px, glyph set, bdfconv map, build
#  mode, weight override)
#   The weight override is None for everything that takes the weight given on the
#   command line. Only the foiling screen's headings differ: they are the same size
#   as the rows below them and are told apart by stroke weight alone, which is the
#   one font property that costs no layout (see fonts/README.md).
#   Everything uses build mode 0 (proportional). Plex Sans has tabular figures --
#   all ten digits share one advance -- so values don't jitter without forcing
#   monospace, and '-', '.' and ':' keep their natural narrow widths. Monospace
#   mode (2), which the stock Inconsolata _mn fonts use, gives punctuation a full
#   digit cell; in a proportional face that reads as "- 2019" and "17: 42: 23".
SPECS = [
    # Net power: between the speed and the plain values, and wide enough for
    # "-2000" in the left column.
    ("plex_net58_tn",   "0", 58, DIGITS, DIGIT_MAP, 0, None),
    # The right column's three values.
    ("plex_big49_tn",   "0", 49, DIGITS_PCT, DIGIT_PCT_MAP, 0, None),
    ("plex_mid30_tn",   "0", 30, DIGITS, DIGIT_MAP, 0, None),
    ("plex_small14_tf", "T", 14, ASCII,  ASCII_MAP, 0, None),
    # Two points smaller, for the foiling screen's four tables. A separate blob
    # rather than shrinking the shared one: the dashboard's layout is tuned around
    # a 14px cap and every constant in `render/dashboard.rs` derives from it.
    ("plex_small12_tf", "T", 12, ASCII,  ASCII_MAP, 0, None),
    # The foiling screen's table headings, at the same 12px cap as its rows: the
    # tables are read by finding a heading first, and at this size a heavier stroke
    # separates them from the parameter labels without costing a pixel of layout.
    #
    # 600 and not 700, which is as bold as Plex goes: at this cap a bold `r` arm
    # touches the following `n`, and the Turn table's heading reads as "Tum". The
    # gap survives at 600 and is gone by 650. Checked by eye at 6x -- no test can
    # see two glyphs merge, because both are drawn exactly where the font says.
    ("plex_semi12_tf",  "T", 12, ASCII,  ASCII_MAP, 0, 600),
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


def glyph_maxima(bdf):
    """Largest width and height of any single glyph in a BDF.

    Not the same thing as bdfconv's `CalculateMaxBBX`, which reports the *union*
    of every glyph box -- `max(y_off + h) - min(y_off)` and the same in x. The
    union is one or two pixels larger than the tallest glyph whenever the
    highest-reaching glyph is not also the deepest-descending one, which for a
    text font is always. A u8g2 glyph stores its own width and height in those
    bit fields, so the per-glyph maximum is what has to fit; comparing the union
    rejects perfectly good fonts. That is what used to make a 12px cap look
    impossible: bdfconv had correctly given `bbx.h` five bits for a 16px glyph,
    and this check compared it against a 17px union.
    """
    widths, heights = [], []
    with open(bdf) as handle:
        for line in handle:
            if line.startswith("BBX "):
                _, w, h, _, _ = line.split()
                widths.append(int(w))
                heights.append(int(h))
    if not widths:
        sys.exit("%s: no glyphs found; refusing to ship an unverified font" % bdf)
    return max(widths), max(heights)


def check_bitfields(name, verbose_output, bdf):
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

    # Per-glyph maxima, not bdfconv's union box -- see glyph_maxima.
    max_w, max_h = glyph_maxima(bdf)
    for what, value, nbits in (("width", max_w, bits["w"]),
                               ("height", max_h, bits["h"]),
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

    ttf, default_weight, outdir = sys.argv[1], float(sys.argv[2]), sys.argv[3]
    only = sys.argv[4:]
    unknown = [name for name in only if name not in [spec[0] for spec in SPECS]]
    if unknown:
        sys.exit("no such blob: %s" % ", ".join(unknown))
    os.makedirs(outdir, exist_ok=True)
    total = 0

    for name, probe, target, glyphs, cmap, mode, override in SPECS:
        if only and name not in only:
            continue
        weight = default_weight if override is None else float(override)
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
        check_bitfields(name, out, bdf)
        subprocess.run([sys.executable, C2BIN, cfile, blob],
                       check=True, stdout=subprocess.DEVNULL)

        size = os.path.getsize(blob)
        total += size
        print("%-16s em=%-3d w=%-3d %s height %2d px (target %2d)  %5d B"
              % (name, em, weight, probe, got, target, size))
        os.remove(bdf)
        os.remove(cfile)

    print("total: %d B" % total)


if __name__ == "__main__":
    main()
