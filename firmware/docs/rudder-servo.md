# Rudder Servo (Back-Foil Angle) Behavior

The rudder controller drives the back-foil angle with a stepper motor
(11HS12-0674D-PG14, 13.73:1 planetary gearbox) through a TMC2209 driver.
Position is open-loop step counting; the only absolute reference is the
mechanical home stop, found by driving into it (open-loop homing). The driver
runs SpreadCycle throughout for torque at speed, so there is no StallGuard
stall detection. All logic lives in
[`app/src/servo_rudder.rs`](../app/src/servo_rudder.rs).

## Wiring (rudder controller board)

| MCU pin | Function |
| --- | --- |
| PC12 / PD2 | UART5 TX / RX to the TMC2209 single-wire UART (115200 baud, driver config + diagnostics only) |
| PC11 | STEP |
| PC10 | DIR |
| PB5 | ENABLE (active low) |
| PB4 | DIAG (StallGuard output disabled; high on driver error) |
| PB3 | INDEX (unused) |

## CAN interface

See [CAN_MESSAGES.md](../../CAN_MESSAGES.md) for the byte layouts.

| ID | Message | Direction | Notes |
| --- | --- | --- | --- |
| 0x010 | ServoRudderSetpoint | to rudder controller | u16 LE, 1000–2000. Out-of-range values are rejected and do **not** feed the watchdog. |
| 0x020 | ServoRudderStatus | from rudder controller | Every 100 ms: state, current setpoint, actual position (setpoint units), fault cause. |
| 0x021 | ServoRudderCommand | to rudder controller | 0 = Initialize. Starts (re-)homing from **any** state. |
| 0x339 | ThrottleState | from the throttle | Armed at power-up → no homing (see Transitions). |

Design rule: once homed, the servo never takes authority away from the
autopilot. A timeout or a stall is reported on 0x020, and the servo keeps
following setpoints as well as it can. Parking the foil somewhere else does not
make a flying boat any safer.


## State machine

| State | Meaning | Setpoints | Motor |
| --- | --- | --- | --- |
| 0 Uninitialized | The first second after power-up, before homing starts; or held after a power-up with the throttle armed. No absolute position known. | Ignored | Driver disabled (free); energized, holding, when held for an armed throttle |
| 1 Operational | Homed, following setpoints. | Followed | Energized (hold current at standstill) |
| 2 Homing | Running the homing sequence. | Ignored (latest one is picked up when Operational) | Energized, homing current (`IRUN_HOMING`) |
| 3 FailSafe | Setpoint watchdog expired; holding the last setpoint. | The next valid one resumes Operational | Energized, holding |
| 4 Fault | Homing failed (see fault cause below). | Ignored (latched) | Disabled |

While Operational or FailSafe, a nonzero fault cause of StallDuringMove means
DIAG tripped during a move since the last homing (with StallGuard off, a driver
error). The position is approximate until the next Initialize.

Transitions:

- **Power-up** → Homing by itself after 1 s (`BOOT_HOME_DELAY`); no
  Initialize is needed. The foil moves without a command, so keep clear of the
  mechanism at power-on. If the driver is not ready (e.g. motor supply off),
  this ends in Fault and needs an Initialize once it is.
- **Power-up with the throttle armed** (ThrottleState 0x339 = Armed within
  that 1 s): no homing. The board reset under way, and homing would swing the
  foil through its whole travel. The driver is configured and energized at
  once, so the foil holds where it is, and the state stays Uninitialized
  (setpoints ignored, position unknown) until an Initialize homes it. A driver
  that does not configure → Fault, driver disabled. No ThrottleState at all
  (throttle off, bench) homes as usual.
- **Initialize (0x021)** from any state → Homing, whatever the throttle
  state. This is the only way out of Fault or Uninitialized, and the only
  thing that clears a StallDuringMove flag.
- **Homing success** → Operational with the fault cause cleared. The watchdog
  starts immediately.
- **Homing failure** → Fault, driver disabled.
- **Watchdog**: in Operational, every valid setpoint re-arms a 2 s timer.
  On expiry the servo goes to FailSafe and holds. A move already in flight
  finishes at the last setpoint rather than stopping dead. The next valid
  setpoint returns it to Operational; no Initialize is needed.
- **Stall while moving** (DIAG trips): the move is abandoned and the fault
  cause becomes StallDuringMove (sticky until the next homing), but the state
  does not change. The servo keeps following setpoints on its step count, and
  the next setpoint retries the move. If the mechanism is really jammed, every
  retry stalls again and the count drifts further, which is why the flag
  stays set.

## Homing sequence (open loop)

1. Read `IFCNT` over UART (3 attempts). No response →
   Fault(DriverNoUartResponse), driver stays disabled.
2. Write GCONF (SpreadCycle), CHOPCONF, IHOLD_IRUN (homing current),
   TCOOLTHRS and SGTHRS (0: StallGuard off); read `IFCNT` again and require a
   delta of exactly 5 → otherwise Fault(DriverError).
3. Enable the driver and step 1.2× the full travel toward the home stop
   (the setpoint-2000 end), ramping from 200 steps/s to the cruise speed.
   From anywhere in the travel that ends against the stop, the motor slipping at the homing current
   for the remainder. While still below 600 steps/s, DRV_STATUS is checked
   every 8 steps: two consecutive open-load samples → Fault(DriverOpenLoad).
   (Each read pauses stepping ~1.2 ms, too long at full speed.) This takes
   ~7.6 s.
4. The end of the budget is taken as the stop. HomingTimeout is no longer
   produced.
5. Back off `BACKOFF_STEPS` from the stop; that position is defined as
   position 0 = setpoint 2000. Switch to the normal run current →
   Operational.

## Fault causes (status byte 5)

| Value | Cause | Typical reason / field action |
| --- | --- | --- |
| 0 | None | — |
| 1 | StallDuringMove | DIAG tripped during a move. With StallGuard off this means a driver error (overtemperature, short). Reported while Operational/FailSafe; the servo keeps tracking with an approximate position. Send Initialize to re-reference. |
| 2 | HomingTimeout | No longer produced: homing is open loop. (Was: no StallGuard stall found.) |
| 3 | DriverNoUartResponse | TMC2209 not answering: no driver power (VM), UART wiring, or wrong slave address. |
| 4 | DriverError | UART works but register writes did not stick (IFCNT mismatch) or a write failed. |
| 5 | DriverOpenLoad | Coil open at the home stop (SG_RESULT reads 0 and DIAG trips): motor unplugged or a broken phase. |

## Motion profile

STEP pulses are software-timed (embassy timer, 30.5 µs tick, with the
fractional remainder carried so the average rate is exact). Moves use a
constant-acceleration ramp: start at 200 full steps/s (60 motor RPM),
accelerate at 5000 steps/s² to 4000 steps/s (1200 motor RPM, ~524°/s at the
gearbox output), and brake symmetrically into the target (~0.76 s and ~1600
steps each way). Full travel takes ~6.8 s. A new setpoint retargets a move in flight; one behind
the direction of travel brakes to the start speed first (overshooting if
needed) and then reverses, instead of reversing at speed. At the ends of travel
a move stops hard rather than run into a stop. Homing ramps the same way to
the cruise speed.

## Tuning constants

All in one block at the top of `app/src/servo_rudder.rs`.

| Constant | Value | Meaning / how to tune |
| --- | --- | --- |
| `IRUN` | 31 | Run current at full scale, ~1.06 A rms (~1.5 A sine peak): ~1.58× the rated 0.67 A, ~2.5× rated copper loss while moving. Raised because the motor ran cold at the low duty cycle; watch motor temp on long moves. |
| `IRUN_HOMING` | 31 | ~1.06 A rms, equal to `IRUN` for now: lower values did not move the wing. Separate so it can be reduced if the stop jams. |
| `IHOLD` | 4 | Standstill hold current (~30% of run). |
| `MRES_FULLSTEP` | 8 | CHOPCONF mres for full steps; intpol still interpolates to 256, so torque is unchanged. Every step-counted constant and step rate scales with it. |
| `SGTHRS` | 0 | StallGuard off (it only works in StealthChop; the driver runs SpreadCycle). |
| `TRAVEL_STEPS` | 24000 | Full steps for full setpoint travel (2000 at home → 1000 at the far stop). Bench-tuned (2026-09). |
| `BACKOFF_STEPS` | 25 | Steps backed off the stop after homing; position 0 lives here. |
| `HOME_DIR_LEVEL` | Low | DIR level that moves toward the home stop. **Verify on hardware first.** |
| `HOMING_SPEED` | = `CRUISE_SPEED` | Open-loop homing speed, reached with the same ramp from `START_SPEED`, at `IRUN_HOMING`. The backoff runs at `START_SPEED`. |
| `START_SPEED` / `CRUISE_SPEED` / `ACCEL` | 200 / 4000 steps/s, 5000 steps/s² | Constant-acceleration move profile (full steps). Bench-tuned (2026-09): ACCEL 8000+ stalled at any cruise speed with motor and driver cool; 5000 steps/s was too fast. |
| `WATCHDOG_TIMEOUT` | 2 s | Setpoint watchdog. |
| `BOOT_HOME_DELAY` | 1 s | Delay before the automatic power-up homing, and how long an Armed ThrottleState (every 200 ms) is waited for. |

## Bring-up checklist

1. No driver power: status shows Uninitialized; Initialize must produce
   Fault(DriverNoUartResponse) — verifies the fault path and UART timeout.
2. Driver powered, motor unplugged: Initialize must produce
   Fault(DriverOpenLoad) within a few hundred ms of homing starting — open-load
   detection is the only thing stopping a dead motor from "homing".
3. Verify `HOME_DIR_LEVEL` moves toward the intended stop; scope PC11 for the
   ramp and clean pulses if in doubt.
4. On the mechanics: home (= setpoint 2000), sweep setpoints
   (`cansend can0 010#D007` = 2000 at home, `010#E803` = 1000 at the far stop), calibrate `TRAVEL_STEPS`, then verify watchdog
   (stop sending → FailSafe after 2 s, holding; resume sending → Operational).
   Check homing from the far end still reaches the home stop (it must not lose
   steps on the way at `IRUN_HOMING`), and that the foil comes free of the
   stop after the backoff.
