# eoi-foil-view

The datalogger's LCD: what the hydrofoils are doing, at 25 fps. Too fast for the e-ink
displays, so it lives here.

- **Side view**: the hull at its pitch and ride height, the struts with the breach and
  target marks, and the foils at their AoA (the rear with a dashed ghost at its command).
- **Stern view**: the hull at its roll with a dashed ghost at the roll demand, the front
  wings by span, and the raw strut height sensors.
- **Last 10 s**: height and target, pitch and its demand, and all three wing AoAs,
  shaded where the flight controller was in MANUAL.

A wing at its servo stop turns red. Anything not received for 0.5 s shows "—". Without
a ride height (on the bench) the hull sits at rest height but still follows pitch and
roll; only with no attitude at all is it a dashed ghost. It is a port of the bottom strip of the video
overlay in python-data-analysis (`overlay/render.py`, c58198e), in the brand colours.

## Where the numbers come from

| Shown | CAN | Notes |
|---|---|---|
| roll, pitch | `0x250` | |
| roll and pitch demand | `0x275` | `hydrofoils.lua` since 2026-10-23; "—" in MANUAL |
| pitch demand, older logs | `0x271` | 2 Hz, height mode only, used when no `0x275` arrives; no roll demand |
| ride height | `0x251` bytes 2–3 | the EKF height the loop controls on |
| target | `0x270` | 2 Hz, so it may go stale for up to 1.5 s |
| MANUAL / FBWA | `0x251` byte 6 | |
| FL / FR AoA, at stop | `0x252` | PWM through the calibration in [boat.toml](boat.toml) |
| rear AoA and command | `0x020` | stepper position and setpoint; only while Operational |
| FL / FR sensor | `0x011` / `0x012` | |

**The front-wing calibration is `SERVO_LIMITS` from `hydrofoils.lua`.** If the script's
limits change, update [boat.toml](boat.toml), or pass an edited copy with `--boat`.

## Running it locally

No CAN bus or screen needed:

```sh
# One frame of the built-in demo, 31 s in (front-left wing at its stop)
cargo run -p eoi-foil-view -- --demo --png out.png --at 31

# A real log, drawn as it was 120 s after --from
cargo run -p eoi-foil-view -- --replay candump.log --from 600 --png out.png --at 120

# In a window (needs SDL2; WSLg works)
cargo run -p eoi-foil-view --features window -- --demo --window
cargo run -p eoi-foil-view --features window -- --replay candump.log --speed 2 --window
```

Through a virtual CAN bus, which tests the same socket path the Pi uses:

```sh
sudo modprobe vcan && sudo ip link add dev vcan0 type vcan && sudo ip link set up vcan0
cargo run -p eoi-foil-view -- --write-demo demo.log
canplayer -I demo.log vcan0=can0 -l i &
cargo run -p eoi-foil-view --features window -- -c vcan0 --window
# or, without a window: listen 15 s, then save what it saw
cargo run -p eoi-foil-view -- -c vcan0 --png out.png --at 15
```

## Tests

```sh
cargo test -p eoi-foil-view
SCREENSHOT_DIR=/tmp/shots cargo test -p eoi-foil-view   # also saves each frame
```

The tests cover:
- the calibrations, checked against the Python overlay's numbers
- staleness and the history
- the candump parser and the demo, decoded through `eoi-can-decoder`
- whole frames, checked by which colours appear in which region. Anti-aliasing can
  differ between machines, so the tests don't compare hashes.

## On the datalogger

`build-and-send.sh` cross-builds it, and `support/eoi-can-display.service` runs it on
`can0`, drawing to `/dev/fb0` at the framebuffer's own size. Any 16, 24 or 32-bit pixel
layout works. `setup-sd.sh` sets 32-bit, because 16-bit makes the gradients band.

Every 10 s it logs what a frame costs to draw (`journalctl -u eoi-can-display`).

A screenshot from the boat's own bus, written over SSH while the service is running:

```sh
./eoi-foil-view --png shot.png --at 10
```

## Fonts

[Raleway](assets/Raleway.ttf), SIL Open Font License ([assets/OFL.txt](assets/OFL.txt)),
the brand book's text face, unmodified. The overlay's Clash Display may not be
redistributed, so the big numbers use Raleway Medium here.
