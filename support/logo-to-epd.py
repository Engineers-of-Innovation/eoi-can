#!/usr/bin/env python3
"""Turn the EoI logo into a ready-to-send framebuffer for the 5.79" e-paper panel.

The display boards paint this on the battery's shutdown warning, with only the
time before the 24 V dies to do it in. So nothing is rendered on the MCU: the
output is the exact 26928 bytes `Epd5in79` clocks out, and the firmware passes
them straight from flash to the panel.

Layout is the panel's own, upright: 792x272, 99 bytes per row, MSB-first, a set
bit is **white** (the controller's RAM polarity, see `epd5in79`). The firmware
derives the 180-degree version for an inverted panel at compile time.

The logo is cropped to its ink, scaled to fit inside a margin, centred, and
thresholded. A preview PNG is written next to the output for checking by eye.

Usage: logo-to-epd.py <logo.png> <out.bin>
   eg: support/logo-to-epd.py eoi-logo--monochrome-black.png \\
           firmware/app/assets/shutdown-logo.bin
"""
import sys

from PIL import Image

WIDTH, HEIGHT = 792, 272
# Clear space around the logo on every side, in panel pixels.
MARGIN = 36
# Scaled pixels darker than this become ink. The logo is solid black on
# transparent, so only the anti-aliased edges are in between.
INK_THRESHOLD = 128


def main(src_path, out_path):
    src = Image.open(src_path).convert("RGBA")
    # Transparent background -> white, so the alpha edge blends to grey.
    flat = Image.new("RGBA", src.size, (255, 255, 255, 255))
    flat.alpha_composite(src)
    grey = flat.convert("L")
    grey = grey.crop(Image.eval(grey, lambda p: 255 - p).getbbox())

    scale = min((WIDTH - 2 * MARGIN) / grey.width, (HEIGHT - 2 * MARGIN) / grey.height)
    size = (round(grey.width * scale), round(grey.height * scale))
    logo = grey.resize(size, Image.LANCZOS)

    canvas = Image.new("L", (WIDTH, HEIGHT), 255)
    canvas.paste(logo, ((WIDTH - size[0]) // 2, (HEIGHT - size[1]) // 2))
    mono = canvas.point(lambda p: 255 if p >= INK_THRESHOLD else 0, "1")

    # Mode "1" packs MSB-first with 1 = white, which is the panel's layout as is.
    data = mono.tobytes()
    assert len(data) == WIDTH * HEIGHT // 8, len(data)
    with open(out_path, "wb") as f:
        f.write(data)
    mono.save(out_path.rsplit(".", 1)[0] + "-preview.png")
    print(f"{out_path}: logo {size[0]}x{size[1]} on {WIDTH}x{HEIGHT}, {len(data)} bytes")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
