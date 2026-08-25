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
    0x261 [16, s]    the cursor cell, re-requested once the cursor settles --
                     which is how a listener infers where the cursor is. `s` is
                     2 when the walk below has run into the parameter's min/max
    0x263 [slot, ..] one frame per configuration slot -- the datalogger's job, not
                     the flight controller's, but sent from here so the whole
                     screen can be driven from one script
    0x264 [action..] a configuration keypress, every few cycles, cycling through
                     stored / restored / undone / factory reset / saved to flash

Then it walks one parameter a step at a time, as holding `+` on the tuner would,
so the status line's "increased from X to Y" can be watched behaving. The walk is
clamped to the parameter's own min/max and reports ST_CLAMPED when it sticks,
exactly as `handle_set` does -- which is how the "clamped at max" wording gets
exercised. Pick the cell and the step size to reach a stop quickly:

    support/send-foil-params.py vcan0 --walk ROLL_LIMIT_DEG --coarse

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
PROTO_VERSION = 9
ST_OK, ST_CLAMPED = 0, 2

# The datalogger's half: the configuration slots and what was last done to one.
# Sent from here too, so the whole foiling screen can be driven from one script.
ID_SLOT, ID_EVENT = 0x263, 0x264
SLOT_EMPTY, SLOT_AT, SLOT_NO_FIX = 0, 1, 2
ACT_STORED, ACT_RESTORED, ACT_UNDONE, ACT_FACTORY, ACT_FLASH = 1, 2, 3, 4, 5

# What the nine slots hold, by slot number as printed on the screen. Three states
# so the column shows all of them at once; the rest are empty.
SLOTS = {
    1: (SLOT_AT, 14, 32),
    2: (SLOT_AT, 9, 15),
    3: (SLOT_NO_FIX, 0, 0),
}

# The events cycled through, one every EVENT_EVERY dumps, so each wording of the
# status line appears in turn.
EVENTS = [
    (ACT_STORED, 4, SLOT_AT, 16, 5),
    (ACT_RESTORED, 2, SLOT_AT, 9, 15),
    (ACT_UNDONE, 0, SLOT_EMPTY, 0, 0),
    (ACT_FACTORY, 0, SLOT_EMPTY, 0, 0),
    (ACT_FLASH, 0, SLOT_EMPTY, 0, 0),
]
EVENT_EVERY = 4

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
    "HYD_RKP": 0.4, "HYD_RSCALE": 0.8,
    "HYD_RSCHED": 493.0, "HYD_FRNTFF": 0.2,
    "SCR_USER1": 0.0, "SCR_USER2": 0.0, "SCR_USER3": 0.0, "SCR_USER4": 0.0,
    "TRN_ENABLE": 0.0, "TRN_REV": 0.0,
}

# The cell the cursor sits on, and the parameter the walk moves.
CURSOR_PARAM = "PTCH_RATE_P"
# Steps to take before turning around, and frames to hold against a stop once the
# walk has reached one -- enough to read the status line, not so many that the
# screen looks frozen.
WALK_STEPS = 8
CLAMPED_FRAMES = 2


def load_params():
    """(index, name, value, min, max, fine, coarse) per parameter, from the
    display's own export. The range comes along because the flight controller
    clamps to it, and a stand-in that did not would never produce a clamped
    status."""
    out = []
    with open(CSV, newline="") as handle:
        for row in csv.DictReader(handle):
            if not row["index"]:
                continue        # config slots, and the pitch-only gap
            lo, hi = float(row["min"]), float(row["max"])
            fine, coarse = float(row["step_fine"]), float(row["step_coarse"])
            # A combined cell lists both halves: `22+23` against two names.
            names = row["param"].split("+")
            for index, name in zip(row["index"].split("+"), names):
                if any(existing[0] == int(index) for existing in out):
                    continue    # the axis table lists each row per axis
                value = TUNE.get(name)
                if value is None:
                    value = round((lo + hi) / 2, int(row["decimals"] or 0))
                out.append((int(index), name, value, lo, hi, fine, coarse))
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

    def send(self, can_id, data):
        self.sock.send(struct.pack("=IB3x8s", can_id, len(data), data))

    def value(self, index, value, status=ST_OK):
        self.send(ID_VALUE, bytes([index, status]) + struct.pack("<f", float(value)))

    def slot(self, slot, state, hour=0, minute=0):
        self.send(ID_SLOT, bytes([slot, state, hour, minute]))

    def event(self, action, slot, state, hour, minute):
        self.send(ID_EVENT, bytes([action, slot, state, hour, minute]))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("channel", nargs="?", default="vcan0")
    ap.add_argument("--once", action="store_true",
                    help="one dump and one cursor frame, then exit")
    ap.add_argument("--period", type=float, default=2.0,
                    help="seconds between dumps (default 2.0; the display times a "
                         "value out after 5 s, so keep this under that)")
    ap.add_argument("--walk", default=CURSOR_PARAM, metavar="PARAM",
                    help=f"the parameter the cursor sits on and the walk moves "
                         f"(default {CURSOR_PARAM})")
    ap.add_argument("--coarse", action="store_true",
                    help="walk in coarse steps, which reaches a min/max -- and so "
                         "the clamped status -- in a few frames")
    ap.add_argument("--event-every", type=int, default=EVENT_EVERY, metavar="N",
                    help=f"send a configuration event every N dumps (default "
                         f"{EVENT_EVERY}; 1 to see them one after another)")
    args = ap.parse_args()

    params = load_params()
    bus = Bus(args.channel)
    walked = next((p for p in params if p[1] == args.walk), None)
    if walked is None:
        sys.exit(f"no parameter named {args.walk!r}; see the param column of "
                 f"{os.path.relpath(CSV)}")
    cursor_index, _, _, lo, hi, fine, coarse = walked
    step_size = coarse if args.coarse else fine

    # Live state, not the table's defaults. The walk below edits this and the dump
    # reads from it, so the two always agree -- a dump that re-sent the original
    # value would snap the walked parameter back on every cycle, which is not
    # something a flight controller does and reads on the display as the value
    # undoing itself.
    current = {p[0]: p[2] for p in params}
    order = [p[0] for p in params]
    step, walk_up, clamped_for = 0, True, 0
    print(f"{args.channel}: {len(params)} parameters, cursor on {args.walk} "
          f"(index {cursor_index}), walking {step_size:g} at a time "
          f"within [{lo:g}, {hi:g}]")

    cycle = 0
    while True:
        bus.value(IDX_VERSION, PROTO_VERSION)
        for index in order:
            bus.value(index, current[index])
            time.sleep(0.002)       # paced, as the flight controller paces a dump
        bus.value(IDX_ALL, len(order))

        # The slot column, re-sent every cycle: the display keeps no configuration
        # state of its own, so a slot it stops hearing about goes back to `empty`.
        for slot in range(1, 10):
            bus.slot(slot, *SLOTS.get(slot, (SLOT_EMPTY, 0, 0)))

        # The cursor cell, re-requested once the cursor settles, so it arrives on
        # its own after the dump -- which is how a listener infers where it is.
        # Walking it a step at a time exercises the status line; it turns around
        # rather than wrapping, so both "increased" and "decreased" appear and the
        # value never jumps.
        time.sleep(0.2)
        target = round(current[cursor_index] + (step_size if walk_up else -step_size), 4)
        # The same clamp `handle_set` applies, reported the same way: the ack
        # carries the value that stuck, and the status says it was not the one
        # asked for.
        value = min(max(target, lo), hi)
        status = ST_OK if value == target else ST_CLAMPED
        current[cursor_index] = value
        bus.value(cursor_index, value, status)
        # One line per walk step: which value went out and whether it stuck, so a
        # screen showing something unexpected can be checked against what was sent.
        print(f"  {args.walk} -> {value:g}"
              f"{' (clamped)' if status == ST_CLAMPED else ''}", flush=True)

        if status == ST_CLAMPED:
            # Hold against the stop for a couple of frames -- long enough to read
            # the line, and the second press is the one that moves nothing at all
            # -- then walk back.
            clamped_for += 1
            if clamped_for == CLAMPED_FRAMES:
                walk_up, step, clamped_for = not walk_up, 0, 0
        else:
            clamped_for = 0
            step += 1
            if step == WALK_STEPS:
                walk_up, step = not walk_up, 0

        # And now and then a configuration keypress, which is the datalogger's to
        # report. Sent last: it takes the status line off the walk, and the display
        # holds it there for a few seconds against the read-backs a restore causes.
        every = max(1, args.event_every)
        if cycle % every == every - 1:
            action, slot, state, hour, minute = EVENTS[(cycle // every) % len(EVENTS)]
            bus.event(action, slot, state, hour, minute)
            print(f"  event: action {action}"
                  f"{f' on config {slot}' if slot else ''}", flush=True)
        cycle += 1

        if args.once:
            return
        time.sleep(args.period)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
