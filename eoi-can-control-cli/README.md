# eoi-can-control-cli

CLI to control EoI boat systems over CAN. Currently it controls the servo
rudder controller (see [`firmware/docs/rudder-servo.md`](../firmware/docs/rudder-servo.md)
and [`CAN_MESSAGES.md`](../CAN_MESSAGES.md)); more subsystems can be added as
new subcommand groups later.

## Usage

```
eoi-can-control-cli [-i can0] rudder <command>
```

All commands follow the controller's status broadcasts (0x020): they show what
the controller is actually doing and exit non-zero when it reports a fault.

| Command | What it does |
| --- | --- |
| `rudder set <1000-2000>` | Holds the setpoint by re-sending it (default 10 Hz) until Ctrl-C, showing the incoming state/position on a live line. Exits non-zero when the controller faults or keeps ignoring setpoints (Uninitialized/FailSafe — run `init` first). `--once` sends a single frame and prints the next status instead; `--rate <hz>` changes the frequency. |
| `rudder init` | Sends Initialize (0x021), which (re)starts homing — the only way out of FailSafe/Fault — then waits for the result: state transitions are printed as they arrive, `Operational` exits 0, a fault (e.g. `DriverNoUartResponse`) exits non-zero with the cause. Homing can take a minute; `--timeout <secs>` (default 90) bounds the wait, `--no-wait` restores fire-and-forget. |
| `rudder status` | Prints decoded status frames (0x020: state, setpoint, actual position, fault cause). `--once` prints the first one and exits. |
| `rudder sweep` | Ramps the setpoint back and forth between `--from`/`--to` (default 1000/2000) as a triangle wave, `--period` seconds per cycle (default 10). Runs until Ctrl-C, or `--cycles <n>`. Monitors status like `set`. |
| `rudder interactive` | Keyboard control: ←/→ nudge the setpoint by `--step` (default 10), Home/End jump to the ends, `i` sends Initialize, `q`/Esc quits. Shows the latest status on a live line. |

`set`, `sweep`, and `interactive` also accept `--init`: home first (the full
Initialize-and-wait sequence) and start the command as soon as homing
completes.

Examples:

```sh
eoi-can-control-cli rudder init                      # home the servo first
eoi-can-control-cli rudder set 1500                  # hold mid-travel until Ctrl-C
eoi-can-control-cli rudder set 1500 --init           # home, then hold in one go
eoi-can-control-cli -i vcan0 rudder set 1500 --once  # single frame, e.g. for watchdog testing
eoi-can-control-cli rudder sweep --from 1200 --to 1800 --period 6 --cycles 3
```

## The 2 s watchdog

The firmware fail-safes (parks the rudder at setpoint 1000) whenever no valid
setpoint arrives for 2 seconds. That is why `set`, `sweep`, and `interactive`
keep re-sending frames, and why exiting them (Ctrl-C) lets the rudder park —
that is the firmware's designed behavior, not an error. Recover with
`rudder init`.
