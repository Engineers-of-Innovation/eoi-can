#!/usr/bin/env python3
"""Convert bdfconv's C output into the raw .u8g2font blob the crate expects.

bdfconv emits `const uint8_t name[N] = "...\\16\\2...";` — a C string literal with
octal escapes. The u8g2-fonts crate's .u8g2font files are just those bytes raw.

Usage: c2u8g2font.py <in.c> <out.u8g2font>
"""
import re
import sys

SIMPLE = {"n": 10, "t": 9, "r": 13, "b": 8, "f": 12, "v": 11, "a": 7,
          "0": 0, "\\": 92, '"': 34, "'": 39, "?": 63}


def decode(literal):
    out = bytearray()
    i = 0
    while i < len(literal):
        c = literal[i]
        if c != "\\":
            out.append(ord(c))
            i += 1
            continue
        i += 1
        c = literal[i]
        if c in "01234567":
            # Octal escape: up to 3 digits.
            j = i
            while j < len(literal) and j - i < 3 and literal[j] in "01234567":
                j += 1
            out.append(int(literal[i:j], 8) & 0xFF)
            i = j
        elif c == "x":
            j = i + 1
            while j < len(literal) and literal[j] in "0123456789abcdefABCDEF":
                j += 1
            out.append(int(literal[i + 1:j], 16) & 0xFF)
            i = j
        elif c in SIMPLE:
            out.append(SIMPLE[c])
            i += 1
        else:
            raise ValueError("unhandled escape \\%s" % c)
    return bytes(out)


def main():
    src = open(sys.argv[1]).read()

    declared = re.search(r"\[(\d+)\]", src)
    declared = int(declared.group(1)) if declared else None

    # Collect every "..." chunk after the '=' and concatenate, as C would.
    body = src[src.index("=") + 1:]
    chunks = re.findall(r'"((?:[^"\\]|\\.)*)"', body)
    data = b"".join(decode(c) for c in chunks)

    # bdfconv's declared length counts the implicit NUL terminator.
    if declared is not None and len(data) == declared - 1:
        pass
    elif declared is not None and len(data) != declared:
        print("warning: decoded %d bytes, header declares %d"
              % (len(data), declared), file=sys.stderr)

    open(sys.argv[2], "wb").write(data)
    print("%s: %d bytes (glyph_count=%d, bbox %dx%d, ascent=%d)"
          % (sys.argv[2], len(data), data[0], data[9], data[10], data[13]))


if __name__ == "__main__":
    main()
