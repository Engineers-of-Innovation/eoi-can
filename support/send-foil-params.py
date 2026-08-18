#!/usr/bin/env python3
"""Stand in for the flight controller: broadcast a foil parameter table on CAN.

Speaks the `0x261 PARAM_VALUE` half of `foil_tune.lua`'s protocol, so the helm
display can be driven without ArduPilot, a boat, or the tuning keyboard. Useful
for `--layout foiling` against `vcan0`, and for the real panel over `can0`.

    support/send-foil-params.py                 # vcan0, loops until Ctrl-C
    support/send-foil-params.py can0            # the real bus
    support/send-foil-params.py vcan0 --once    # one dump, then exit

What it sends, in the order the flight controller would:

    0x261 [0xFE, 0]  proto version 7
    0x261 [idx, 0]   one frame per parameter, a whole-table dump
    0x261 [0xFF, 0]  end-of-dump marker, value = entries sent
    0x261 [16, 0]    the cursor cell, re-requested once the cursor settles --
                     which is how a listener infers where the cursor is

Then it walks PTCH_RATE_P upwards a step at a time, as holding `+` on the tuner
would, so the status line's "increased from X to Y" can be watched behaving.

Which parameters exist, and which index each has, come from
FOILING_PARAMETERS.csv -- the same file the display's tests check themselves
against, so this cannot drift from the screen. The *values* are the real derived
tune (tools/foil_derive.py and hydrofoils.lua in the ardupilot repo, TUNE_REV 21)
where there is one, and the midpoint of the parameter's range otherwise.

Raw AF_CAN rather than python-can: nothing to install, and this is a test aid.
"""
import argparse
import csv
import os
import socket
import struct
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CSV = os.path.join(os.path.dirname(HERE), "FOILING_PARAMETERS.csv")

ID_VALUE = 0x261
IDX_VERSION, IDX_ALL = 0xFE, 0xFF
PROTO_VERSION = 7
ST_OK = 0

# The derived tune, by ArduPilot name. Rate gains from `tools/foil_derive.py`
# ("PASTE INTO hydrofoils.lua section 1b"); the height and rear loops from
# `hydrofoils.lua`'s own constants at TUNE_REV 21. Anything absent falls back to
# the midpoint of its min/max, which is enough to fill a cell.
TUNE = {
    "PTCH_RATE_FF": 2.93, "PTCH_RATE_P": 4.05, "PTCH_RATE_I": 3.82,
    "RLL_RATE_FF": 0.33, "RLL_RATE_P": 0.44, "RLL_RATE_I": 0.43,
    "PTCH_RATE_D": 0.0, "RLL_RATE_D": 0.0,
    "PTCH_LIM_MAX_DEG": 2.0, "PTCH_LIM_MIN_DEG": -3.0, "ROLL_LIMIT_DEG": 10.0,
    "HYD_KP": 960.0, "HYD_KI": 96.0, "HYD_KD": 1800.0, "HYD_IMAX": 150.0,
    "HYD_TARGET": 0.30, "HYD_ARM": 2.4,
    "HYD_CMDMAX": 2.0, "HYD_CMDMIN": -3.0,
    "HYD_RKP": 0.4, "HYD_RSCALE": 0.8, "HYD_RSCHED": 493.0, "HYD_FRNTFF": 0.2,
    "SCR_USER1": 0.0, "SCR_USER2": 0.0, "SCR_USER3": 0.0, "SCR_USER4": 0.0,
    "TRN_ENABLE": 0.0, "TRN_REV": 0.0,
}

# The cell the cursor sits on, and the parameter the walk moves.
CURSOR_PARAM = "PTCH_RATE_P"
# One fine step of PTCH_RATE_P, from foil_tune.lua's table, and how many to take
# before turning around.
FINE_STEP = 0.02
FINE_STEPS = 8


def load_params():
    """(index, name, value) per parameter, from the display's own export."""
    out = []
    with open(CSV, newline="") as handle:
        for row in csv.DictReader(handle):
            if not row["index"]:
                continue        # config slots, and the pitch-only gap
            # A combined cell lists both halves: `22+23` against two names.
            names = row["param"].split("+")
            for index, name in zip(row["index"].split("+"), names):
                if any(existing[0] == int(index) for existing in out):
                    continue    # the axis table lists each row per axis
                value = TUNE.get(name)
                if value is None:
                    lo, hi = float(row["min"]), float(row["max"])
                    value = round((lo + hi) / 2, int(row["decimals"] or 0))
                out.append((int(index), name, value))
    return sorted(out)


class Bus:
    def __init__(self, channel):
        self.sock = socket.socket(socket.AF_CAN, socket.SOCK_RAW, socket.CAN_RAW)
        try:
            self.sock.bind((channel,))
        except OSError as exc:
            sys.exit(f"cannot bind {channel}: {exc}\n"
                     f"  vcan0:  sudo ip link add dev vcan0 type vcan && "
                     f"sudo ip link set up vcan0\n"
                     f"  can0:   sudo ip link set can0 up type can bitrate 1000000")

    def value(self, index, value, status=ST_OK):
        data = bytes([index, status]) + struct.pack("<f", float(value))
        self.sock.send(struct.pack("=IB3x8s", ID_VALUE, len(data), data))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("channel", nargs="?", default="vcan0")
    ap.add_argument("--once", action="store_true",
                    help="one dump and one cursor frame, then exit")
    ap.add_argument("--period", type=float, default=2.0,
                    help="seconds between dumps (default 2.0; the display times a "
                         "value out after 5 s, so keep this under that)")
    args = ap.parse_args()

    params = load_params()
    bus = Bus(args.channel)
    cursor_index = next(i for i, name, _ in params if name == CURSOR_PARAM)

    # Live state, not the table's defaults. The walk below edits this and the dump
    # reads from it, so the two always agree -- a dump that re-sent the original
    # value would snap the walked parameter back on every cycle, which is not
    # something a flight controller does and reads on the display as the value
    # undoing itself.
    current = {index: value for index, _, value in params}
    order = [index for index, _, _ in params]
    step, walk_up = 0, True
    print(f"{args.channel}: {len(params)} parameters, cursor on {CURSOR_PARAM} "
          f"(index {cursor_index})")

    while True:
        bus.value(IDX_VERSION, PROTO_VERSION)
        for index in order:
            bus.value(index, current[index])
            time.sleep(0.002)       # paced, as the flight controller paces a dump
        bus.value(IDX_ALL, len(order))

        # The cursor cell, re-requested once the cursor settles, so it arrives on
        # its own after the dump -- which is how a listener infers where it is.
        # Walking it a fine step at a time exercises the status line; it turns
        # around rather than wrapping, so both "increased" and "decreased" appear
        # and the value never jumps.
        time.sleep(0.2)
        if step == FINE_STEPS:
            walk_up, step = not walk_up, 0
        current[cursor_index] = round(
            current[cursor_index] + (FINE_STEP if walk_up else -FINE_STEP), 2)
        step += 1
        bus.value(cursor_index, current[cursor_index])

        if args.once:
            return
        time.sleep(args.period)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
