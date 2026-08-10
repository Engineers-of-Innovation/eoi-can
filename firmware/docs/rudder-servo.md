# Rudder Servo (Back-Foil Angle) Behavior

The rudder controller drives the back-foil angle with a stepper motor
(11HS12-0674D-PG14, 13.73:1 planetary gearbox) through a TMC2209 driver.
Position is open-loop step counting; the only absolute reference is a
StallGuard-detected mechanical stop found during homing. All logic lives in
[`app/src/servo_rudder.rs`](../app/src/servo_rudder.rs).

## Wiring (rudder controller board)

| MCU pin | Function |
| --- | --- |
| PC12 / PD2 | UART5 TX / RX to the TMC2209 single-wire UART (115200 baud, driver config + diagnostics only) |
| PC11 | STEP |
| PC10 | DIR |
| PB5 | ENABLE (active low) |
| PB4 | DIAG (high on StallGuard stall) |
| PB3 | INDEX (unused) |

## CAN interface

See [CAN_MESSAGES.md](https://github.com/Engineers-of-Innovation/eoi-can/blob/main/CAN_MESSAGES.md)
for the byte layouts.

| ID | Message | Direction | Notes |
| --- | --- | --- | --- |
| 0x010 | ServoRudderSetpoint | to rudder controller | u16 LE, 1000–2000. Out-of-range values are rejected and do **not** feed the watchdog. |
| 0x020 | ServoRudderStatus | from rudder controller | Every 100 ms: state, current setpoint, actual position (setpoint units), fault cause. |
| 0x021 | ServoRudderCommand | to rudder controller | 0 = Initialize. Starts (re-)homing from **any** state. |

## State machine

| State | Meaning | Setpoints | Motor |
| --- | --- | --- | --- |
| 0 Uninitialized | Boot state. No absolute position known. | Ignored | Driver disabled (free) |
| 1 Operational | Homed, following setpoints. | Followed | Energized (hold current at standstill) |
| 2 Homing | Running the homing sequence. | Ignored (latest one is picked up when Operational) | Energized, reduced homing current |
| 3 FailSafe | Setpoint watchdog expired; parked at setpoint 1000. | Ignored (latched) | Energized, holding |
| 4 Fault | See fault cause below. | Ignored (latched) | Holding if the recovery re-home succeeded, disabled otherwise |

Transitions:

- **Initialize (0x021)** from any state → Homing. This is the only way out of
  FailSafe and Fault — recovery is a deliberate operator action.
- **Homing success** → Operational (watchdog starts immediately; if no
  setpoint arrives within 2 s the servo parks in FailSafe).
- **Homing failure** → Fault, driver disabled.
- **Watchdog**: in Operational, every valid setpoint re-arms a 2 s timer.
  On expiry the servo moves (position still trusted) to setpoint 1000 and
  latches FailSafe.
- **Stall while moving** (DIAG trips during a normal or failsafe move): the
  step count can no longer be trusted, so the servo latches
  Fault(StallDuringMove) and immediately re-runs the homing sequence to park,
  holding, at the home/failsafe position. It stays in Fault until Initialize.

## Homing sequence

1. Read `IFCNT` over UART (3 attempts). No response →
   Fault(DriverNoUartResponse), driver stays disabled.
2. Write GCONF, CHOPCONF, IHOLD_IRUN (reduced homing current), TCOOLTHRS and
   SGTHRS; read `IFCNT` again and require a delta of exactly 5 →
   otherwise Fault(DriverError).
3. Enable the driver and step at constant speed toward the home stop
   (the setpoint-1000 end) until DIAG trips. Every ~100 ms the SG_RESULT
   load value is logged over defmt for SGTHRS tuning.
4. If 1.2× the full travel is stepped without a stall →
   Fault(HomingTimeout), driver disabled.
5. Back off `BACKOFF_STEPS` from the stop; that position is defined as
   position 0 = setpoint 1000. Switch to the normal run current →
   Operational.

## Fault causes (status byte 5)

| Value | Cause | Typical reason / field action |
| --- | --- | --- |
| 0 | None | — |
| 1 | StallDuringMove | Mechanism jammed or SGTHRS too sensitive. Servo re-homed itself and holds at 1000. Clear the jam, send Initialize. |
| 2 | HomingTimeout | No stall found: motor unplugged, driver unpowered, SGTHRS too insensitive, or wrong `HOME_DIR_LEVEL`. |
| 3 | DriverNoUartResponse | TMC2209 not answering: no driver power (VM), UART wiring, or wrong slave address. |
| 4 | DriverError | UART works but register writes did not stick (IFCNT mismatch) or a write failed. |

## Motion profile

STEP pulses are software-timed (embassy timer, 30.5 µs resolution) with a
linear ramp: start ~400 Hz, accelerate to ~2 kHz cruise, symmetric
deceleration into the target. At 8 microsteps and the 13.73:1 gearbox this is
roughly 33°/s at the foil shaft. A new setpoint retargets a move in flight;
a direction reversal restarts the ramp from standstill.

## Tuning constants

All in one block at the top of `app/src/servo_rudder.rs`.

| Constant | Value | Meaning / how to tune |
| --- | --- | --- |
| `IRUN` | 13 | Run current, ~0.47 A rms = 0.67 A sine peak (rated). Raise toward 19 only if torque is short, watching motor temp. |
| `IRUN_HOMING` | 8 | Reduced current while grinding into the stop (gearbox multiplies stall torque 13.7×). |
| `IHOLD` | 4 | Standstill hold current (~30% of run). |
| `MRES_8_MICROSTEPS` | 5 | CHOPCONF mres for 8 microsteps. Lower microstepping = faster foil, coarser resolution. |
| `SGTHRS_HOMING` | 60 | StallGuard threshold; DIAG trips when SG_RESULT < 2×SGTHRS. Tune with the SG_RESULT homing logs: pick ~half the unloaded value. |
| `TRAVEL_STEPS` | 20000 | Microsteps for full setpoint travel (1000→2000). **Calibrate on the real mechanics.** |
| `BACKOFF_STEPS` | 200 | Steps backed off the stop after homing; position 0 lives here. |
| `HOME_DIR_LEVEL` | Low | DIR level that moves toward the home stop. **Verify on hardware first.** |
| `HOMING_DELAY_TICKS` | ~400 Hz | Homing step rate. StallGuard needs enough speed for a usable SG_RESULT signal. |
| `START_DELAY_TICKS` / `MIN_DELAY_TICKS` | 400 Hz / 2 kHz | Ramp start and cruise step rates. |
| `WATCHDOG_TIMEOUT` | 2 s | Setpoint watchdog. |
| `FAILSAFE_SETPOINT` | 1000 | Parking position on watchdog expiry (= home end). |

## Bring-up checklist

1. No driver power: status shows Uninitialized; Initialize must produce
   Fault(DriverNoUartResponse) — verifies the fault path and UART timeout.
2. Driver powered, motor free: Initialize; watch defmt for IFCNT readback and
   SG_RESULT values; stall the shaft by hand and tune `SGTHRS_HOMING` until
   DIAG trips reliably without false positives.
3. Verify `HOME_DIR_LEVEL` moves toward the intended stop; scope PC11 for the
   ramp and clean pulses if in doubt.
4. On the mechanics: home, sweep setpoints (`cansend can0 010#E803` = 1000,
   `010#D007` = 2000), calibrate `TRAVEL_STEPS`, then verify watchdog
   (stop sending → FailSafe after 2 s) and stall recovery (block mid-move).
