use core::sync::atomic::{AtomicI32, AtomicU8, AtomicU16, Ordering};

use defmt::*;
use embassy_futures::join::join;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_stm32::Peri;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::mode::Async;
use embassy_stm32::peripherals::{PB4, PB5, PC10, PC11};
use embassy_stm32::usart::{UartRx, UartTx};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Ticker, Timer};
use eoi_can_decoder::{ServoRudderCommand, ThrottleState};
use tmc2209::reg;
use tmc2209::reg::{ReadableRegister, WritableRegister};

pub const CAN_ID_SERVO_STATUS: StandardId = unsafe { StandardId::new_unchecked(0x20) };

pub const SETPOINT_MIN: u16 = 1000;
pub const SETPOINT_MAX: u16 = 2000;
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(2);
// The servo homes by itself at power-up, after this delay for the motor supply
// to settle. The foil moves without a command, so keep clear at power-on.
// Unless the throttle reports Armed within the delay: then this board reset
// while under way, and it holds the foil where it is instead (`hold_unhomed`).
// The throttle sends its state every 200 ms, so the delay covers ~5 frames.
const BOOT_HOME_DELAY: Duration = Duration::from_secs(1);

// MS1 (AD0) and MS2 (AD1) are strapped to 3V3 on the board, so the driver
// listens on UART slave address 3.
const TMC_ADDR: u8 = 3;
pub const TMC_BAUD: u32 = 115_200;
const TMC_READ_TIMEOUT: Duration = Duration::from_millis(20);

// Motor: 11HS12-0674D-PG14, 0.67 A/phase, 200 full-steps/rev, 13.73:1 gearbox.
// Driver: 0.1 ohm external sense resistors, vsense=1 -> full scale ~1.06 A rms.
// IRUN 31 = full scale -> ~1.06 A rms (~1.5 A sine peak) at vsense=1:
// ~1.58x the rated 0.67 A, so ~2.5x the rated copper loss while moving.
// Raised from 19 via 26 and 28 because the motor stayed cold at the low duty
// cycle (bench, 2026-09); it is only worth it while the motor stays cool.
// TODO(bench): watch MotorTemp on long moves. More needs vsense=0 (~1.9 A rms
// full scale), beyond this motor.
const IRUN: u8 = 31;
// IRUN_HOMING 31 -> ~1.06 A rms, equal to IRUN for now: at 14 homing did not
// move the wing (bench, 2026-09). Kept separate so it can be lowered again.
const IRUN_HOMING: u8 = 24;
const IHOLD: u8 = 4;
const IHOLD_DELAY: u8 = 8;
// Full steps (mres 8). CHOPCONF intpol (on in the reset value) still
// interpolates to 256 microsteps, so the coil currents and torque are the same
// as at any microstep setting; only the angle per STEP pulse changes. Every
// step-counted constant below is in full steps, halved from the 2-microstep
// values, and the step rates halved with them, so the motion is unchanged.
const MRES_FULLSTEP: u32 = 8;
// TMC2209 CHOPCONF reset value (toff=3, hstrt=5, tbl=2, intpol=1); writing
// CHOPCONF from all-zeroes would set toff=0 and disable the driver.
const CHOPCONF_RESET: u32 = 0x1000_0053;

// The driver runs SpreadCycle throughout (GCONF en_spread_cycle) for torque at
// speed, and StallGuard only works in StealthChop, so there is no stall
// detection: SGTHRS 0 keeps the StallGuard output off DIAG, which then only
// reports driver errors (overtemperature, short to ground). Homing is open
// loop, see `home_inner`. TCOOLTHRS and SGTHRS are still written so the
// IFCNT check in `configure_driver` covers a fixed five writes.
const SGTHRS: u32 = 0;
const TCOOLTHRS_VAL: u32 = 0xF_FFFF;
// Ignore DIAG for the first steps of a move.
const STALL_BLANK_STEPS: u32 = 16;

// Full travel (setpoint 2000 at home .. 1000 at the far stop) in full steps.
// TODO(bench): calibrate on the real mechanics: home, then drive slowly into
// the far stop and take the position from the "Stall detected at position"
// log message.
// Bench (2026-09): tuned up from 18_000, 20_000 and 22_000.
const TRAVEL_STEPS: i32 = 24_000;
// TODO(bench): verify the backoff clears the stop with enough margin that
// normal moves to setpoint 2000 never re-touch it.
const BACKOFF_STEPS: i32 = 25; // = 50 at 2 microsteps; same angle
const HOMING_BUDGET_PERCENT: u32 = 120;
// Which DIR level moves toward the home stop.
// TODO(bench): verify before first homing; if wrong, the foil runs to the
// far stop and faults with HomingTimeout.
const HOME_DIR_LEVEL: Level = Level::Low;

// Step rates are software-timed on the 32.768 kHz embassy tick (~30.5 us).
const TICK_HZ: u64 = embassy_time::TICK_HZ;
// Move profile, in full steps: start at START_SPEED, accelerate at a constant
// ACCEL to CRUISE_SPEED, brake symmetrically. The old ramp took one tick off
// the step delay every 2 steps: ~120 steps/s^2 at the bottom but ~15000 at the
// top, exactly where a stepper has least torque. ACCEL 5000 steps/s^2 covers
// 200 -> 3000 steps/s in ~0.56 s / ~900 steps. Bench (2026-09): 8000 and
// 10000 stalled at any cruise speed (the rotor falls behind while ramping),
// with motor and driver both cool, so it is acceleration, not heat.
const START_SPEED: f32 = 300.0; // steps/s, 60 motor RPM
// 3000 steps/s = 900 motor RPM = ~393 deg/s at the gearbox output (13.73:1).
// Bench (2026-09): ran well with ACCEL up to 5000; 5000 steps/s was too fast.
// Lowered from 4000 (2026-10) because the motor stalled again: a stepper has
// more torque at lower speed. The step period is ~11 ticks here, so single
// steps alternate 305/336 us (the carry keeps the average exact), and any task
// holding the executor for more than ~0.6 ms costs sync.
const CRUISE_SPEED: f32 = 3000.0; // steps/s
const ACCEL: f32 = 5000.0; // steps/s^2
// Open-loop homing ramps from START_SPEED at ACCEL to HOMING_SPEED and holds
// it into the stop, at the reduced IRUN_HOMING current. It must not lose steps
// on the way: that would silently put home short of the stop. The backoff off
// the stop runs at START_SPEED.
const HOMING_SPEED: f32 = CRUISE_SPEED;
// Open-load check during homing: a DRV_STATUS read pauses stepping ~1.2 ms,
// under one step at 600 steps/s but several at full speed (a stepper loses
// sync beyond two). So it only runs in the slow start of the ramp, which still
// catches an unplugged motor; the at-stop check covers the rest.
// Every 8: at ACCEL 5000 the ramp spends only ~32 steps below the limit, and
// the open-load rule needs two consecutive samples.
const STATUS_SAMPLE_EVERY_STEPS: u32 = 8;
const STATUS_SAMPLE_MAX_SPEED: f32 = 600.0; // steps/s
const STEP_PULSE_CYCLES: u32 = 40; // ~500 ns high at 80 MHz (datasheet min 100 ns)

pub static SERVO_SETPOINT: Signal<CriticalSectionRawMutex, u16> = Signal::new();
pub static SERVO_COMMAND: Signal<CriticalSectionRawMutex, ServoRudderCommand> = Signal::new();
pub static THROTTLE_STATE: Signal<CriticalSectionRawMutex, ThrottleState> = Signal::new();

static STATE: AtomicU8 = AtomicU8::new(State::Uninitialized as u8);
static FAULT_CAUSE: AtomicU8 = AtomicU8::new(FaultCause::None as u8);
static CURRENT_SETPOINT: AtomicU16 = AtomicU16::new(SETPOINT_MAX); // home
static POSITION_STEPS: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, PartialEq, Format)]
#[repr(u8)]
enum State {
    Uninitialized = 0,
    Operational = 1,
    Homing = 2,
    FailSafe = 3,
    Fault = 4,
}

#[derive(Clone, Copy, PartialEq, Format)]
#[repr(u8)]
enum FaultCause {
    None = 0,
    StallDuringMove = 1,
    // No longer produced (homing is open loop); kept so 2 stays reserved on CAN.
    #[allow(dead_code)]
    HomingTimeout = 2,
    DriverNoUartResponse = 3,
    DriverError = 4,
    DriverOpenLoad = 5,
}

enum MoveResult {
    Reached,
    Stalled,
    Initialize,
}

enum TmcError {
    Uart,
    Timeout,
}

// Home (step 0) is setpoint 2000 and the far stop is 1000, matching the
// autopilot's PWM sense for this foil (bench, 2026-09): steps count away from
// home as the setpoint falls.
fn setpoint_to_steps(setpoint: u16) -> i32 {
    let units = SETPOINT_MAX - setpoint.clamp(SETPOINT_MIN, SETPOINT_MAX);
    units as i32 * TRAVEL_STEPS / (SETPOINT_MAX - SETPOINT_MIN) as i32
}

fn steps_to_setpoint(steps: i32) -> u16 {
    let units = (steps * (SETPOINT_MAX - SETPOINT_MIN) as i32 / TRAVEL_STEPS)
        .clamp(0, (SETPOINT_MAX - SETPOINT_MIN) as i32);
    SETPOINT_MAX - units as u16
}

fn away_level() -> Level {
    match HOME_DIR_LEVEL {
        Level::Low => Level::High,
        Level::High => Level::Low,
    }
}

pub fn init(
    step_pin: Peri<'static, PC11>,
    dir_pin: Peri<'static, PC10>,
    enable_pin: Peri<'static, PB5>,
    diag_pin: Peri<'static, PB4>,
) -> (
    Output<'static>,
    Output<'static>,
    Output<'static>,
    Input<'static>,
) {
    let step = Output::new(step_pin, Level::Low, Speed::Medium);
    let dir = Output::new(dir_pin, Level::Low, Speed::Low);
    // Enable is active-low: start with the driver disabled.
    let enable = Output::new(enable_pin, Level::High, Speed::Low);
    let diag = Input::new(diag_pin, Pull::Down);
    (step, dir, enable, diag)
}

struct Tmc2209Uart {
    tx: UartTx<'static, Async>,
    rx: UartRx<'static, Async>,
}

impl Tmc2209Uart {
    async fn write<R: WritableRegister>(&mut self, register: R) -> Result<(), TmcError> {
        let request = tmc2209::write_request(TMC_ADDR, register);
        self.tx
            .write(request.bytes())
            .await
            .map_err(|_| TmcError::Uart)
    }

    async fn read<R: ReadableRegister>(&mut self) -> Result<R, TmcError> {
        let request = tmc2209::read_request::<R>(TMC_ADDR);
        let deadline = Instant::now() + TMC_READ_TIMEOUT;
        let mut reader = tmc2209::Reader::default();
        let mut buffer = [0u8; 24];

        // Arm RX together with TX: the reply starts only ~8 bit times after
        // our request ends, so it must not race the RX DMA setup. The RX line
        // also sees our own request (single-wire UART); parse_response skips
        // it by only syncing on replies addressed to the master.
        match select(
            join(
                self.rx.read_until_idle(&mut buffer),
                self.tx.write(request.bytes()),
            ),
            Timer::at(deadline),
        )
        .await
        {
            Either::First((rx_result, tx_result)) => {
                tx_result.map_err(|_| TmcError::Uart)?;
                match rx_result {
                    Ok(len) => {
                        if let Some(value) = parse_response::<R>(&mut reader, &buffer[..len]) {
                            return Ok(value);
                        }
                    }
                    Err(e) => trace!("TMC2209 UART rx error: {:?}", e),
                }
            }
            Either::Second(_) => return Err(TmcError::Timeout),
        }

        loop {
            match select(self.rx.read_until_idle(&mut buffer), Timer::at(deadline)).await {
                Either::First(Ok(len)) => {
                    if let Some(value) = parse_response::<R>(&mut reader, &buffer[..len]) {
                        return Ok(value);
                    }
                }
                Either::First(Err(e)) => {
                    trace!("TMC2209 UART rx error: {:?}", e);
                }
                Either::Second(_) => return Err(TmcError::Timeout),
            }
        }
    }
}

fn parse_response<R: ReadableRegister>(reader: &mut tmc2209::Reader, bytes: &[u8]) -> Option<R> {
    if let (_, Some(response)) = reader.read_response(bytes)
        && response.crc_is_valid()
        && let Ok(address) = response.reg_addr()
        && address == R::ADDRESS
    {
        Some(R::from(response.data_u32()))
    } else {
        None
    }
}

struct Servo {
    step: Output<'static>,
    dir: Output<'static>,
    enable: Output<'static>,
    diag: Input<'static>,
    tmc: Tmc2209Uart,
    position: i32,
    watchdog_deadline: Instant,
    state: State,
    /// Why the position is only approximate while tracking: a stall since the
    /// last homing (the step count may be off), or a homing that timed out and
    /// was accepted anyway (`servo-timeout-homes`). None after a clean homing.
    tracking_fault: FaultCause,
}

impl Servo {
    fn set_state(&mut self, state: State, cause: FaultCause) {
        info!("Servo state: {} (fault cause: {})", state, cause);
        self.state = state;
        STATE.store(state as u8, Ordering::Relaxed);
        FAULT_CAUSE.store(cause as u8, Ordering::Relaxed);
    }

    /// Fault cause reported while tracking (Operational / FailSafe): sticky
    /// until the next homing, so the bus knows the position is approximate.
    fn tracking_cause(&self) -> FaultCause {
        self.tracking_fault
    }

    /// A valid setpoint: feed the watchdog and, if it had expired, resume.
    fn accept_setpoint(&mut self, setpoint: u16) {
        self.watchdog_deadline = Instant::now() + WATCHDOG_TIMEOUT;
        CURRENT_SETPOINT.store(setpoint, Ordering::Relaxed);
        if self.state == State::FailSafe {
            info!("Setpoints resumed");
            self.set_state(State::Operational, self.tracking_cause());
        }
    }

    /// Watchdog expiry: report it and hold the last setpoint. Parking the foil
    /// somewhere else would take authority away without making anything safer;
    /// the next valid setpoint resumes tracking.
    fn enter_failsafe(&mut self) {
        warn!("Setpoint watchdog expired; holding last setpoint");
        self.set_state(State::FailSafe, self.tracking_cause());
    }

    /// Stall while tracking: flag it and carry on with the step count as is,
    /// so the autopilot keeps (degraded) authority. The move is abandoned; the
    /// next setpoint retries it.
    fn note_stall(&mut self) {
        warn!("Stall at position {}; continuing on the step count", self.position);
        self.tracking_fault = FaultCause::StallDuringMove;
        self.set_state(self.state, FaultCause::StallDuringMove);
    }

    fn step_pulse(&mut self, direction: i32) {
        self.step.set_high();
        cortex_m::asm::delay(STEP_PULSE_CYCLES);
        self.step.set_low();
        self.position += direction;
        POSITION_STEPS.store(self.position, Ordering::Relaxed);
    }

    async fn read_ifcnt(&mut self) -> Result<u8, FaultCause> {
        for _ in 0..3 {
            if let Ok(ifcnt) = self.tmc.read::<reg::IFCNT>().await {
                return Ok(ifcnt.0 as u8);
            }
        }
        warn!("TMC2209 not responding on UART (IFCNT read failed 3x)");
        Err(FaultCause::DriverNoUartResponse)
    }

    async fn configure_driver(&mut self) -> Result<(), FaultCause> {
        let start_count = self.read_ifcnt().await?;

        let mut gconf = reg::GCONF::default();
        gconf.set_pdn_disable(true);
        gconf.set_mstep_reg_select(true);
        gconf.set_multistep_filt(true);
        gconf.set_en_spread_cycle(true);
        self.tmc
            .write(gconf)
            .await
            .map_err(|_| FaultCause::DriverError)?;

        let mut chopconf = reg::CHOPCONF::from(CHOPCONF_RESET);
        chopconf.set_vsense(true);
        chopconf.set_mres(MRES_FULLSTEP);
        self.tmc
            .write(chopconf)
            .await
            .map_err(|_| FaultCause::DriverError)?;

        self.tmc
            .write(ihold_irun(IRUN_HOMING))
            .await
            .map_err(|_| FaultCause::DriverError)?;

        let mut tcoolthrs = reg::TCOOLTHRS::default();
        tcoolthrs.set(TCOOLTHRS_VAL);
        self.tmc
            .write(tcoolthrs)
            .await
            .map_err(|_| FaultCause::DriverError)?;

        self.tmc
            .write(reg::SGTHRS(SGTHRS))
            .await
            .map_err(|_| FaultCause::DriverError)?;

        let end_count = self.read_ifcnt().await?;
        let delta = end_count.wrapping_sub(start_count);
        if delta != 5 {
            warn!("TMC2209 IFCNT delta {} after 5 writes", delta);
            return Err(FaultCause::DriverError);
        }
        Ok(())
    }

    /// Coil and StallGuard health straight from the driver. Open-load
    /// (ola/olb) means a coil is not conducting (wiring/crimp); cs_actual
    /// shows the current scale actually applied.
    async fn read_driver_status(&mut self, context: &str) -> Option<reg::DRV_STATUS> {
        match self.tmc.read::<reg::DRV_STATUS>().await {
            Ok(s) => {
                info!(
                    "DRV_STATUS ({=str}): stst={} stealth={} cs_actual={} ola={} olb={} s2ga={} s2gb={} s2vsa={} s2vsb={} otpw={} ot={}",
                    context,
                    s.stst(),
                    s.stealth(),
                    s.cs_actual(),
                    s.ola(),
                    s.olb(),
                    s.s2ga(),
                    s.s2gb(),
                    s.s2vsa(),
                    s.s2vsb(),
                    s.otpw(),
                    s.ot()
                );
                Some(s)
            }
            Err(_) => {
                warn!("DRV_STATUS ({=str}): read failed", context);
                None
            }
        }
    }

    /// Open-loop homing: drive into the home stop, back off, define position 0.
    async fn home(&mut self) -> Result<(), FaultCause> {
        let result = self.home_inner().await;
        if result.is_err() {
            // Position is unknown; do not hold torque on it.
            self.enable.set_high();
        }
        result
    }

    async fn home_inner(&mut self) -> Result<(), FaultCause> {
        self.tracking_fault = FaultCause::None;
        self.configure_driver().await?;

        self.enable.set_low();
        Timer::after_millis(10).await;

        info!("DIAG before homing: {}", self.diag.is_high());
        // Log-only: open-load flags are unreliable at standstill.
        self.read_driver_status("pre-homing").await;

        self.dir.set_level(HOME_DIR_LEVEL);
        Timer::after_ticks(1).await;

        // No stall detection, so step the whole budget: from anywhere in the
        // travel that ends against the stop, the motor slipping at reduced
        // current for whatever remains. The margin covers TRAVEL_STEPS being
        // an estimate.
        let budget = TRAVEL_STEPS as u32 * HOMING_BUDGET_PERCENT / 100;
        let mut open_load_samples: u32 = 0;
        // Same constant-acceleration ramp as `move_to`, accelerating only.
        let mut v2 = START_SPEED * START_SPEED;
        let mut speed = START_SPEED;
        let mut carry: f32 = 0.0;
        let mut next = Instant::now();
        for stepped in 1..=budget {
            self.step.set_high();
            cortex_m::asm::delay(STEP_PULSE_CYCLES);
            self.step.set_low();

            if speed < STATUS_SAMPLE_MAX_SPEED && stepped.is_multiple_of(STATUS_SAMPLE_EVERY_STEPS) {
                // Open-load flags can flicker; require two consecutive
                // samples before faulting. Without this an unplugged motor
                // would "home" successfully.
                if let Ok(s) = self.tmc.read::<reg::DRV_STATUS>().await {
                    if s.ola() || s.olb() {
                        open_load_samples += 1;
                        warn!(
                            "Open load during homing (ola={} olb={}, sample {})",
                            s.ola(),
                            s.olb(),
                            open_load_samples
                        );
                        if open_load_samples >= 2 {
                            return Err(FaultCause::DriverOpenLoad);
                        }
                    } else {
                        open_load_samples = 0;
                    }
                }
                // The read paused stepping; restart the schedule from now.
                next = Instant::now();
                carry = 0.0;
            }

            v2 = (v2 + 2.0 * ACCEL).min(HOMING_SPEED * HOMING_SPEED);
            speed = 0.5 * (speed + v2 / speed);
            let period = TICK_HZ as f32 / speed + carry;
            let whole = period as u64;
            carry = period - whole as f32;
            next += Duration::from_ticks(whole);
            let now = Instant::now();
            if next < now {
                next = now;
                carry = 0.0;
            }
            Timer::at(next).await;
        }
        info!("Homing drove {} steps; taking this as the home stop", budget);
        if let Some(s) = self.read_driver_status("at stop").await
            && (s.ola() || s.olb())
        {
            warn!("Open load at home stop (ola={} olb={})", s.ola(), s.olb());
            return Err(FaultCause::DriverOpenLoad);
        }
        Timer::after_millis(50).await;

        self.dir.set_level(away_level());
        Timer::after_ticks(1).await;
        let mut next = Instant::now();
        for _ in 0..BACKOFF_STEPS {
            self.step.set_high();
            cortex_m::asm::delay(STEP_PULSE_CYCLES);
            self.step.set_low();
            next += Duration::from_ticks((TICK_HZ as f32 / START_SPEED) as u64);
            Timer::at(next).await;
        }

        self.position = 0;
        POSITION_STEPS.store(0, Ordering::Relaxed);

        self.tmc
            .write(ihold_irun(IRUN))
            .await
            .map_err(|_| FaultCause::DriverError)?;
        Ok(())
    }

    /// Move toward `target_steps`, retargeting on every new setpoint. If the
    /// watchdog expires mid-move the move still finishes: it holds the last
    /// setpoint rather than stopping dead.
    ///
    /// Constant-acceleration ramp: speed^2 changes by 2 * ACCEL every step, so
    /// the motor gets the same acceleration at every speed. The braking
    /// distance (v^2 - START^2) / (2 * ACCEL) is exact in steps, so the move
    /// slows down in time for the target. A retarget behind the direction of
    /// travel brakes to START_SPEED first, overshooting if it must, and only
    /// then reverses.
    async fn move_to(&mut self, target_steps: i32) -> MoveResult {
        let mut target = target_steps.clamp(0, TRAVEL_STEPS);
        let mut blank = STALL_BLANK_STEPS;
        // Direction of travel: 0 at standstill, +1 away from home, -1 toward it.
        let mut moving: i32 = 0;
        let mut v2 = START_SPEED * START_SPEED;
        let mut speed = START_SPEED;
        // Step period in ticks is fractional; carry the remainder so the
        // average rate is exact despite the 30.5 us tick.
        let mut carry: f32 = 0.0;
        let mut next = Instant::now();

        loop {
            if let Some(command) = SERVO_COMMAND.try_take()
                && command == ServoRudderCommand::Initialize
            {
                return MoveResult::Initialize;
            }
            if let Some(setpoint) = SERVO_SETPOINT.try_take() {
                self.accept_setpoint(setpoint);
                target = setpoint_to_steps(setpoint).clamp(0, TRAVEL_STEPS);
            }
            if self.state == State::Operational && Instant::now() >= self.watchdog_deadline {
                self.enter_failsafe();
            }

            let to_go = target - self.position;
            // Steps still to go in the current direction of travel.
            let ahead = if moving != 0 && to_go.signum() == moving {
                to_go.unsigned_abs()
            } else {
                0
            };
            // Stop once slowed down with nothing ahead, and always at the ends
            // of travel (a hard stop, but never a run into the stops).
            let at_limit = (moving < 0 && self.position <= 0)
                || (moving > 0 && self.position >= TRAVEL_STEPS);
            // Test v2, not speed: v2 is clamped to exactly START^2, while the
            // Newton-updated speed only approaches START from above and may
            // never compare <= in f32, which would run past the target.
            if moving != 0 && ((ahead == 0 && v2 <= START_SPEED * START_SPEED) || at_limit) {
                moving = 0;
            }

            if moving == 0 {
                if to_go == 0 {
                    return MoveResult::Reached;
                }
                moving = to_go.signum();
                self.dir.set_level(if moving > 0 {
                    away_level()
                } else {
                    HOME_DIR_LEVEL
                });
                v2 = START_SPEED * START_SPEED;
                speed = START_SPEED;
                carry = 0.0;
                blank = STALL_BLANK_STEPS;
                Timer::after_ticks(1).await;
                next = Instant::now();
                continue;
            }

            if blank > 0 {
                blank -= 1;
            } else if self.diag.is_high() {
                warn!("DIAG tripped at position {}", self.position);
                return MoveResult::Stalled;
            }

            // Steps needed to brake to START_SPEED from here. This step uses up
            // one of `ahead`, and accelerating adds one to the braking
            // distance, so: accelerate only with two to spare, hold with one,
            // otherwise brake. (Comparing `ahead` to `braking` alone creeps
            // one step past the target at START_SPEED and oscillates.)
            let braking = (v2 - START_SPEED * START_SPEED) / (2.0 * ACCEL);
            let ahead = ahead as f32;
            if ahead >= braking + 2.0 {
                v2 = (v2 + 2.0 * ACCEL).min(CRUISE_SPEED * CRUISE_SPEED);
            } else if ahead < braking + 1.0 {
                v2 = (v2 - 2.0 * ACCEL).max(START_SPEED * START_SPEED);
            }
            // One Newton step from the previous speed: sqrt without libm, and
            // exact to well under a step/s since v2 changes little per step.
            speed = 0.5 * (speed + v2 / speed);

            self.step_pulse(moving);

            let period = TICK_HZ as f32 / speed + carry;
            let whole = period as u64;
            carry = period - whole as f32;
            next += Duration::from_ticks(whole);
            let now = Instant::now();
            if next < now {
                next = now;
                carry = 0.0;
            }
            Timer::at(next).await;
        }
    }

    async fn initialize(&mut self) -> State {
        self.set_state(State::Homing, FaultCause::None);
        match self.home().await {
            Ok(()) => {
                self.watchdog_deadline = Instant::now() + WATCHDOG_TIMEOUT;
                CURRENT_SETPOINT.store(steps_to_setpoint(self.position), Ordering::Relaxed);
                self.set_state(State::Operational, self.tracking_cause());
                State::Operational
            }
            Err(cause) => {
                self.set_state(State::Fault, cause);
                State::Fault
            }
        }
    }

    /// Power-up with the throttle armed: the step count died with the reset, so
    /// there is no position to track from, but homing would swing the foil
    /// through its whole travel under way. Energize the driver so the foil holds
    /// where it is, and stay Uninitialized (setpoints ignored) until an
    /// Initialize homes it.
    async fn hold_unhomed(&mut self) -> State {
        let result = async {
            self.configure_driver().await?;
            // `configure_driver` leaves the homing current set.
            self.tmc
                .write(ihold_irun(IRUN))
                .await
                .map_err(|_| FaultCause::DriverError)
        }
        .await;
        match result {
            Ok(()) => {
                self.enable.set_low();
                self.set_state(State::Uninitialized, FaultCause::None);
                State::Uninitialized
            }
            Err(cause) => {
                self.set_state(State::Fault, cause);
                State::Fault
            }
        }
    }

    /// Uninitialized / Fault: only an Initialize command acts.
    async fn idle_locked(&mut self, state: State) -> State {
        match select(SERVO_COMMAND.wait(), SERVO_SETPOINT.wait()).await {
            Either::First(ServoRudderCommand::Initialize) => self.initialize().await,
            Either::First(_) => {
                warn!("Unknown servo command ignored");
                state
            }
            Either::Second(setpoint) => {
                debug!("Setpoint {} ignored in state {}", setpoint, state);
                state
            }
        }
    }

    /// Operational / FailSafe: follow setpoints. The watchdog only runs while
    /// Operational; in FailSafe the servo holds until a setpoint arrives.
    async fn tracking(&mut self) -> State {
        let deadline = self.watchdog_deadline;
        let operational = self.state == State::Operational;
        let watchdog = async {
            if operational {
                Timer::at(deadline).await
            } else {
                core::future::pending::<()>().await
            }
        };
        match select3(SERVO_SETPOINT.wait(), SERVO_COMMAND.wait(), watchdog).await {
            Either3::First(setpoint) => {
                self.accept_setpoint(setpoint);
                match self.move_to(setpoint_to_steps(setpoint)).await {
                    MoveResult::Reached => {}
                    MoveResult::Stalled => self.note_stall(),
                    MoveResult::Initialize => return self.initialize().await,
                }
                self.state
            }
            Either3::Second(ServoRudderCommand::Initialize) => self.initialize().await,
            Either3::Second(_) => {
                warn!("Unknown servo command ignored");
                self.state
            }
            Either3::Third(()) => {
                self.enter_failsafe();
                self.state
            }
        }
    }
}

fn ihold_irun(irun: u8) -> reg::IHOLD_IRUN {
    let mut register = reg::IHOLD_IRUN::default();
    register.set_ihold(IHOLD);
    register.set_irun(irun);
    register.set_ihold_delay(IHOLD_DELAY);
    register
}

/// Whether the throttle reports Armed before `window` runs out. Returns as soon
/// as it does, so the foil is held with as little free time as possible. No
/// state at all (throttle off, bench) counts as not armed.
async fn throttle_armed_within(window: Duration) -> bool {
    let deadline = Instant::now() + window;
    loop {
        match select(THROTTLE_STATE.wait(), Timer::at(deadline)).await {
            Either::First(ThrottleState::Armed) => return true,
            Either::First(_) => {}
            Either::Second(()) => return false,
        }
    }
}

#[embassy_executor::task]
pub async fn servo_control_task(
    step: Output<'static>,
    dir: Output<'static>,
    enable: Output<'static>,
    diag: Input<'static>,
    uart_tx: UartTx<'static, Async>,
    uart_rx: UartRx<'static, Async>,
) {
    let mut servo = Servo {
        step,
        dir,
        enable,
        diag,
        tmc: Tmc2209Uart {
            tx: uart_tx,
            rx: uart_rx,
        },
        position: 0,
        watchdog_deadline: Instant::now(),
        state: State::Uninitialized,
        tracking_fault: FaultCause::None,
    };
    servo.set_state(State::Uninitialized, FaultCause::None);

    // Home at power-up instead of waiting for Initialize, unless the throttle
    // is armed. An Initialize that arrived during the delay is dropped, or it
    // would re-home right after.
    let armed = throttle_armed_within(BOOT_HOME_DELAY).await;
    SERVO_COMMAND.reset();
    let mut state = if armed {
        warn!("Throttle armed at power-up: not homing, holding position until Initialize");
        servo.hold_unhomed().await
    } else {
        info!("Power-up homing");
        servo.initialize().await
    };

    loop {
        state = match state {
            State::Operational | State::FailSafe => servo.tracking().await,
            State::Homing => core::unreachable!(),
            _ => servo.idle_locked(state).await,
        };
    }
}

#[embassy_executor::task]
pub async fn status_task(mut can_tx: BufferedCanSender) {
    let mut ticker = Ticker::every(Duration::from_millis(100));
    loop {
        let setpoint = CURRENT_SETPOINT.load(Ordering::Relaxed);
        let position = steps_to_setpoint(POSITION_STEPS.load(Ordering::Relaxed));
        let data = [
            STATE.load(Ordering::Relaxed),
            setpoint.to_le_bytes()[0],
            setpoint.to_le_bytes()[1],
            position.to_le_bytes()[0],
            position.to_le_bytes()[1],
            FAULT_CAUSE.load(Ordering::Relaxed),
        ];
        let frame = Frame::new_data(CAN_ID_SERVO_STATUS, &data).unwrap();
        if let Err(e) = can_tx.try_write(frame) {
            warn!("Servo status CAN tx error: {:?}", e);
        }
        ticker.next().await;
    }
}
