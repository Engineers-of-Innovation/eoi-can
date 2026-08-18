#!/usr/bin/env python3
"""Emit the foiling screen's hotkey and cursor map for the datalogger.

The display side -- key, label, screen row, column -- is parsed out of
`draw-display/src/render/foiling.rs` rather than retyped, so this cannot drift
from what is actually drawn. The tuning side -- ArduPilot parameter names,
ranges, steps, and which are locked in flight -- is the boat's spec and lives
here, because the display neither knows nor needs it.

Usage: support/export-foiling-params.py            # writes both files
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SRC = os.path.join(ROOT, "draw-display", "src", "render", "foiling.rs")

# Screen order, left to right. The cursor moves between columns in this order,
# and `FoilColumn` on the wire uses these names.
COLUMNS = ["Pitch", "Roll", "Mid", "Right", "Slot"]

# label -> (pitch param, roll param) for the axis table. `None` means that axis
# has no counterpart, so the cell is empty and the cursor should skip it.
AXIS_PARAMS = {
    "RATE_P":   ("PTCH_RATE_P",        "RLL_RATE_P"),
    "RATE_I":   ("PTCH_RATE_I",        "RLL_RATE_I"),
    "RATE_D":   ("PTCH_RATE_D",        "RLL_RATE_D"),
    "RATE_FF":  ("PTCH_RATE_FF",       "RLL_RATE_FF"),
    "RATE_IMAX":("PTCH_RATE_IMAX",     "RLL_RATE_IMAX"),
    "TCONST":   ("PTCH2SRV_TCONST",    "RLL2SRV_TCONST"),
    "RMAX":     ("PTCH2SRV_RMAX_UP+PTCH2SRV_RMAX_DN", "RLL2SRV_RMAX"),
    "LIMIT":    ("PTCH_LIM_MAX_DEG+PTCH_LIM_MIN_DEG", "ROLL_LIMIT_DEG"),
    "FLT_T":    ("PTCH_RATE_FLTT",     "RLL_RATE_FLTT"),
    "FLT_E":    ("PTCH_RATE_FLTE",     "RLL_RATE_FLTE"),
    "FLT_D":    ("PTCH_RATE_FLTD",     "RLL_RATE_FLTD"),
    "SMAX":     ("PTCH_RATE_SMAX",     "RLL_RATE_SMAX"),
    "RLL>PTCH": ("PTCH2SRV_RLL",       None),
}

# ArduPilot name -> foil_tune.lua PT index, the wire contract for 0x260/0x261/0x262.
# Mirrors tools/foil_tune.py's PARAMS at PROTO_VERSION 7. 13-15 and 46-47 are
# unused; 37-38 are retired (HYD_HSRC, HYD_HDIV) and must never be reused.
INDEX = {
    "RLL_RATE_P": 1, "RLL_RATE_I": 2, "RLL_RATE_D": 3, "RLL_RATE_FF": 4,
    "RLL_RATE_IMAX": 5, "RLL2SRV_TCONST": 6, "RLL2SRV_RMAX": 7,
    "ROLL_LIMIT_DEG": 8, "RLL_RATE_FLTT": 9, "RLL_RATE_FLTE": 10,
    "RLL_RATE_FLTD": 11, "RLL_RATE_SMAX": 12,
    "PTCH_RATE_P": 16, "PTCH_RATE_I": 17, "PTCH_RATE_D": 18, "PTCH_RATE_FF": 19,
    "PTCH_RATE_IMAX": 20, "PTCH2SRV_TCONST": 21, "PTCH2SRV_RMAX_UP": 22,
    "PTCH2SRV_RMAX_DN": 23, "PTCH2SRV_RLL": 24, "PTCH_LIM_MAX_DEG": 25,
    "PTCH_LIM_MIN_DEG": 26, "PTCH_RATE_FLTT": 27, "PTCH_RATE_FLTE": 28,
    "PTCH_RATE_FLTD": 29, "PTCH_RATE_SMAX": 30, "SCALING_SPEED": 31,
    "HYD_KP": 32, "HYD_KI": 33, "HYD_KD": 34, "HYD_IMAX": 35, "HYD_TARGET": 36,
    "HYD_ARM": 39, "HYD_CMDMAX": 52, "HYD_CMDMIN": 53,
    "HYD_RKP": 54, "HYD_RSCALE": 55, "HYD_RSCHED": 56, "HYD_FRNTFF": 57,
    "TRN_ENABLE": 40, "TRN_ON": 41, "TRN_FULL": 42, "TRN_MAX": 43,
    "TRN_RATE": 44, "TRN_REV": 45,
    "SCR_USER1": 48, "SCR_USER2": 49, "SCR_USER3": 50, "SCR_USER4": 51,
}


# (param, min, max, fine, coarse, locked_in_flight)
LIMITS = {
    "PTCH_RATE_P":        (0.02, 8,    0.02,  0.1,  False),
    "RLL_RATE_P":         (0.02, 2,    0.005, 0.02, False),
    "PTCH_RATE_I":        (0,    8,    0.02,  0.1,  False),
    "RLL_RATE_I":         (0,    2,    0.005, 0.02, False),
    "PTCH_RATE_D":        (0,    0.5,  0.001, 0.005, False),
    "RLL_RATE_D":         (0,    0.5,  0.001, 0.005, False),
    "PTCH_RATE_FF":       (0,    4,    0.01,  0.05, False),
    "RLL_RATE_FF":        (0,    3,    0.01,  0.05, False),
    "PTCH_RATE_IMAX":     (0,    40,   0.1,   0.5,  False),
    "RLL_RATE_IMAX":      (0,    30,   0.1,   0.5,  False),
    "PTCH2SRV_TCONST":    (0.1,  2,    0.05,  0.1,  False),
    "RLL2SRV_TCONST":     (0.1,  2,    0.05,  0.1,  False),
    "PTCH2SRV_RMAX_UP":   (0,    180,  5,     15,   False),
    "PTCH2SRV_RMAX_DN":   (0,    180,  5,     15,   False),
    "RLL2SRV_RMAX":       (0,    180,  5,     15,   False),
    "PTCH_LIM_MAX_DEG":   (1,    10,   1,     5,    True),
    "PTCH_LIM_MIN_DEG":   (-10,  -1,   1,     5,    True),
    "ROLL_LIMIT_DEG":     (5,    20,   1,     5,    True),
    "PTCH_RATE_FLTT":     (0,    100,  1,     5,    False),
    "RLL_RATE_FLTT":      (0,    100,  1,     5,    False),
    "PTCH_RATE_FLTE":     (0,    100,  1,     5,    False),
    "RLL_RATE_FLTE":      (0,    100,  1,     5,    False),
    "PTCH_RATE_FLTD":     (0,    100,  1,     5,    False),
    "RLL_RATE_FLTD":      (0,    100,  1,     5,    False),
    "PTCH_RATE_SMAX":     (0,    200,  5,     20,   False),
    "RLL_RATE_SMAX":      (0,    200,  5,     20,   False),
    "PTCH2SRV_RLL":       (0,    1.5,  0.01,  0.05, False),
    "HYD_KP":             (0,    2000, 10,    50,   False),
    "HYD_KI":             (0,    500,  5,     20,   False),
    "HYD_KD":             (0,    2000, 10,    50,   False),
    "HYD_IMAX":           (0,    500,  10,    50,   False),
    "HYD_TARGET":         (0,    1,    0.01,  0.05, False),
    "HYD_CMDMAX":         (0.5,  5,    0.1,   0.5,  False),
    "HYD_CMDMIN":         (-8,  -0.5,  0.1,   0.5,  False),
    "HYD_ARM":            (0,    3.8,  0.05,  0.2,  False),
    "HYD_RKP":            (0.15, 1.2,  0.02,  0.1,  False),
    "HYD_RSCALE":         (0.5,  1.2,  0.02,  0.1,  False),
    "HYD_RSCHED":         (0,    1200, 5,     25,   False),
    "HYD_FRNTFF":         (0,    0.5,  0.01,  0.05, False),
    "TRN_ENABLE":         (0,    1,    1,     1,    False),
    "TRN_ON":             (5,    60,   1,     5,    False),
    "TRN_FULL":           (10,   100,  1,     5,    False),
    "TRN_MAX":            (0,    20,   0.5,   2,    False),
    "TRN_RATE":           (1,    20,   0.5,   2,    False),
    "TRN_REV":            (0,    1,    1,     1,    True),
    "SCR_USER1":          (0,    1,    1,     1,    True),
    "SCR_USER2":          (-10,  10,   0.5,   1,    False),
    "SCR_USER3":          (-20,  20,   1,     5,    False),
    "SCR_USER4":          (0,    2100, 10,    50,   True),
    "SCALING_SPEED":      (4,    15,   0.5,   1,    False),
}

SINGLE_PARAMS = {
    "KP": "HYD_KP", "KI": "HYD_KI", "KD": "HYD_KD", "IMAX": "HYD_IMAX",
    "TARGET": "HYD_TARGET", "CMD": "HYD_CMDMAX+HYD_CMDMIN", "ARM": "HYD_ARM",
    "RKP": "HYD_RKP", "RSCALE": "HYD_RSCALE", "RSCHED": "HYD_RSCHED",
    "FRNTFF": "HYD_FRNTFF",
    "ENABLE": "TRN_ENABLE", "ON": "TRN_ON", "FULL": "TRN_FULL",
    "MAX": "TRN_MAX", "RATE": "TRN_RATE", "REV": "TRN_REV",
    "MODE": "SCR_USER1", "TEST_P": "SCR_USER2", "TEST_R": "SCR_USER3",
    "JOG": "SCR_USER4",
    "SPEED": "SCALING_SPEED",
}


def parse_source():
    text = open(SRC).read()

    def table(name):
        block = re.search(r"const " + name + r": \[Row; \d+\] = \[(.*?)\n\];",
                          text, re.S).group(1)
        return re.findall(r'row\("([^"]+)",\s*"([^"]+)",\s*(\d+)\)', block)

    blocks = {}
    for const in ("MID_BLOCKS", "RIGHT_BLOCKS"):
        chunk = re.search(r"const " + const + r".*?\n\];", text, re.S).group(0)
        for group, rows_const, first in re.findall(
                r'name: "(\w+)",\s*rows: &(\w+),\s*first_row: (\d+)', chunk):
            blocks[group] = (rows_const, int(first))
    return {n: table(n) for n in
            ("AXIS", "HEIGHT", "REAR", "TURN", "MODE", "GLOBAL")}, blocks


def limits_for(param):
    """Range and step for a cell, taking the union when it is an up/down pair."""
    halves = param.split("+")
    known = [LIMITS[p] for p in halves if p in LIMITS]
    if not known:
        return ("", "", "", "", "")
    lo = min(k[0] for k in known)
    hi = max(k[1] for k in known)
    fine = known[0][2]
    coarse = known[0][3]
    locked = any(k[4] for k in known)
    return (lo, hi, fine, coarse, "yes" if locked else "")


def main():
    tables, blocks = parse_source()
    cells = []

    for index, (key, label, decimals) in enumerate(tables["AXIS"]):
        row = 1 + index
        pitch, roll = AXIS_PARAMS[label]
        for column, param in (("Pitch", pitch), ("Roll", roll)):
            if param is None:
                cells.append((key, column, row, label, "AXIS", "", "", "", "",
                              "", "", decimals, "skip"))
                continue
            lo, hi, fine, coarse, locked = limits_for(param)
            cells.append((key, column, row, label, "AXIS", param, lo, hi, fine,
                          coarse, locked, decimals, "adjust"))

    for group, column in (("HEIGHT", "Mid"), ("REAR", "Mid"),
                          ("TURN", "Right"), ("MODE", "Right"),
                          ("GLOBAL", "Right")):
        rows_const, first = blocks[group]
        for index, (key, label, decimals) in enumerate(tables[rows_const]):
            param = SINGLE_PARAMS[label]
            lo, hi, fine, coarse, locked = limits_for(param)
            cells.append((key, column, first + index, label, group, param, lo,
                          hi, fine, coarse, locked, decimals, "adjust"))

    for slot in range(1, 10):
        cells.append((str(slot), "Slot", slot, "config %d" % slot, "CONFIG", "",
                      "", "", "", "", "", "", "short_restore;long_store"))
    cells.append(("~", "Slot", 10, "undo", "CONFIG", "", "", "", "", "", "", "",
                  "undo"))
    cells.append(("0", "Slot", 11, "factory", "CONFIG", "", "", "", "", "", "",
                  "", "factory_reset"))

    header = ("key,column,col_index,row,label,group,param,index,min,max,"
              "step_fine,step_coarse,locked_in_flight,decimals,action")
    lines = [header]
    for (key, column, row, label, group, param, lo, hi, fine, coarse, locked,
         decimals, action) in cells:
        index = "+".join(str(INDEX[p]) for p in param.split("+")) if param else ""
        lines.append(",".join(str(v) for v in (
            key, column, COLUMNS.index(column), row, label, group, param, index,
            lo, hi, fine, coarse, locked, decimals, action)))

    out = os.path.join(ROOT, "FOILING_PARAMETERS.csv")
    with open(out, "w") as handle:
        handle.write("\n".join(lines) + "\n")
    print("%s: %d cells" % (os.path.relpath(out, ROOT), len(cells)))
    return cells, tables, blocks


if __name__ == "__main__":
    main()
