use core::sync::atomic::{AtomicI32, AtomicU8, AtomicU16, Ordering};

use defmt::*;
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
use eoi_can_decoder::ServoRudderCommand;
use tmc2209::reg;
use tmc2209::reg::{ReadableRegister, WritableRegister};

pub const CAN_ID_SERVO_STATUS: StandardId = unsafe { StandardId::new_unchecked(0x20) };

pub const SETPOINT_MIN: u16 = 1000;
pub const SETPOINT_MAX: u16 = 2000;
const FAILSAFE_SETPOINT: u16 = 1000;
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(2);

const TMC_ADDR: u8 = 0;
pub const TMC_BAUD: u32 = 115_200;
const TMC_READ_TIMEOUT: Duration = Duration::from_millis(20);

// Motor: 11HS12-0674D-PG14, 0.67 A/phase, 200 full-steps/rev, 13.73:1 gearbox.
// Driver: 0.1 ohm external sense resistors, vsense=1 -> full scale ~1.06 A rms.
// IRUN 13 -> ~0.47 A rms (0.67 A sine peak = rated phase current).
// TODO(bench): raise IRUN (max 19 = rated rms heating) only if torque is
// short under real load, watching motor temperature.
const IRUN: u8 = 13;
const IRUN_HOMING: u8 = 8;
const IHOLD: u8 = 4;
const IHOLD_DELAY: u8 = 8;
const MRES_8_MICROSTEPS: u32 = 5;
// TMC2209 CHOPCONF reset value (toff=3, hstrt=5, tbl=2, intpol=1); writing
// CHOPCONF from all-zeroes would set toff=0 and disable the driver.
const CHOPCONF_RESET: u32 = 0x1000_0053;

// StallGuard: DIAG trips when SG_RESULT < 2*SGTHRS.
// TODO(bench): tune with the "Homing SG_RESULT" defmt logs (lower = more
// load): pick roughly half the unloaded value so a hand-stall trips DIAG
// reliably without false positives during normal homing.
const SGTHRS_HOMING: u32 = 60;
const TCOOLTHRS_VAL: u32 = 0xF_FFFF;
// Ignore DIAG for the first steps of a move (StallGuard is unreliable while
// accelerating from standstill).
const STALL_BLANK_STEPS: u32 = 32;

// Full travel (setpoint 1000..2000) in microsteps.
// TODO(bench): calibrate on the real mechanics: home, then drive slowly into
// the far stop and take the position from the "Stall detected at position"
// log message.
const TRAVEL_STEPS: i32 = 20_000;
// TODO(bench): verify the backoff clears the stop with enough margin that
// normal moves to setpoint 1000 never re-touch it.
const BACKOFF_STEPS: i32 = 200;
const HOMING_BUDGET_PERCENT: u32 = 120;
// Which DIR level moves toward the home stop.
// TODO(bench): verify before first homing; if wrong, the foil runs to the
// far stop and faults with HomingTimeout.
const HOME_DIR_LEVEL: Level = Level::Low;

// Step rates are software-timed on the 32.768 kHz embassy tick (~30.5 us).
const TICK_HZ: u64 = embassy_time::TICK_HZ;
const START_DELAY_TICKS: u64 = TICK_HZ / 400; // ~400 Hz ramp start
// TODO(bench): ~2 kHz cruise = ~33 deg/s at the foil shaft (8 microsteps,
// 13.73:1 gearbox); drop MRES to 4 microsteps if the rudder must be faster.
const MIN_DELAY_TICKS: u64 = TICK_HZ / 2000; // ~2 kHz cruise
// TODO(bench): StallGuard needs enough speed for a usable SG_RESULT signal;
// raise the homing rate if SG_RESULT sits near zero while running free.
const HOMING_DELAY_TICKS: u64 = TICK_HZ / 400;
const ACCEL_EVERY_N_STEPS: u32 = 4;
const SG_SAMPLE_EVERY_STEPS: u32 = 40; // ~100 ms at homing speed
const STEP_PULSE_CYCLES: u32 = 40; // ~500 ns high at 80 MHz (datasheet min 100 ns)

pub static SERVO_SETPOINT: Signal<CriticalSectionRawMutex, u16> = Signal::new();
pub static SERVO_COMMAND: Signal<CriticalSectionRawMutex, ServoRudderCommand> = Signal::new();

static STATE: AtomicU8 = AtomicU8::new(State::Uninitialized as u8);
static FAULT_CAUSE: AtomicU8 = AtomicU8::new(FaultCause::None as u8);
static CURRENT_SETPOINT: AtomicU16 = AtomicU16::new(FAILSAFE_SETPOINT);
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
    HomingTimeout = 2,
    DriverNoUartResponse = 3,
    DriverError = 4,
}

#[derive(PartialEq)]
enum MoveMode {
    /// Setpoints retarget the move and feed the watchdog.
    Tracking,
    /// Only an Initialize command can interrupt (failsafe move).
    Fixed,
}

enum MoveResult {
    Reached,
    Stalled,
    Initialize,
    WatchdogExpired,
}

enum TmcError {
    Uart,
    Timeout,
}

fn setpoint_to_steps(setpoint: u16) -> i32 {
    let units = setpoint.clamp(SETPOINT_MIN, SETPOINT_MAX) - SETPOINT_MIN;
    units as i32 * TRAVEL_STEPS / (SETPOINT_MAX - SETPOINT_MIN) as i32
}

fn steps_to_setpoint(steps: i32) -> u16 {
    let units = (steps * (SETPOINT_MAX - SETPOINT_MIN) as i32 / TRAVEL_STEPS)
        .clamp(0, (SETPOINT_MAX - SETPOINT_MIN) as i32);
    SETPOINT_MIN + units as u16
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
        self.tx
            .write(request.bytes())
            .await
            .map_err(|_| TmcError::Uart)?;

        // The RX line also sees our own request (single-wire UART); the
        // Reader only syncs on replies addressed to the master.
        let deadline = Instant::now() + TMC_READ_TIMEOUT;
        let mut reader = tmc2209::Reader::default();
        let mut buffer = [0u8; 24];
        loop {
            match select(self.rx.read_until_idle(&mut buffer), Timer::at(deadline)).await {
                Either::First(Ok(len)) => {
                    if let (_, Some(response)) = reader.read_response(&buffer[..len])
                        && response.crc_is_valid()
                        && let Ok(address) = response.reg_addr()
                        && address == R::ADDRESS
                    {
                        return Ok(R::from(response.data_u32()));
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

struct Servo {
    step: Output<'static>,
    dir: Output<'static>,
    enable: Output<'static>,
    diag: Input<'static>,
    tmc: Tmc2209Uart,
    position: i32,
    watchdog_deadline: Instant,
}

impl Servo {
    fn set_state(&self, state: State, cause: FaultCause) {
        info!("Servo state: {} (fault cause: {})", state, cause);
        STATE.store(state as u8, Ordering::Relaxed);
        FAULT_CAUSE.store(cause as u8, Ordering::Relaxed);
    }

    fn step_pulse(&mut self, direction: i32) {
        self.step.set_high();
        cortex_m::asm::delay(STEP_PULSE_CYCLES);
        self.step.set_low();
        self.position += direction;
        POSITION_STEPS.store(self.position, Ordering::Relaxed);
    }

    async fn configure_driver(&mut self) -> Result<(), FaultCause> {
        let mut start_count = None;
        for _ in 0..3 {
            if let Ok(ifcnt) = self.tmc.read::<reg::IFCNT>().await {
                start_count = Some(ifcnt.0 as u8);
                break;
            }
        }
        let Some(start_count) = start_count else {
            return Err(FaultCause::DriverNoUartResponse);
        };

        let mut gconf = reg::GCONF::default();
        gconf.set_pdn_disable(true);
        gconf.set_mstep_reg_select(true);
        gconf.set_multistep_filt(true);
        self.tmc
            .write(gconf)
            .await
            .map_err(|_| FaultCause::DriverError)?;

        let mut chopconf = reg::CHOPCONF::from(CHOPCONF_RESET);
        chopconf.set_vsense(true);
        chopconf.set_mres(MRES_8_MICROSTEPS);
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
            .write(reg::SGTHRS(SGTHRS_HOMING))
            .await
            .map_err(|_| FaultCause::DriverError)?;

        match self.tmc.read::<reg::IFCNT>().await {
            Ok(ifcnt) => {
                let delta = (ifcnt.0 as u8).wrapping_sub(start_count);
                if delta != 5 {
                    warn!("TMC2209 IFCNT delta {} after 5 writes", delta);
                    return Err(FaultCause::DriverError);
                }
            }
            Err(_) => return Err(FaultCause::DriverNoUartResponse),
        }
        Ok(())
    }

    /// Stall-seek toward the home stop, back off, and define position 0.
    async fn home(&mut self) -> Result<(), FaultCause> {
        let result = self.home_inner().await;
        if result.is_err() {
            // Position is unknown; do not hold torque on it.
            self.enable.set_high();
        }
        result
    }

    async fn home_inner(&mut self) -> Result<(), FaultCause> {
        self.configure_driver().await?;

        self.enable.set_low();
        Timer::after_millis(10).await;

        self.dir.set_level(HOME_DIR_LEVEL);
        Timer::after_ticks(1).await;

        let budget = TRAVEL_STEPS as u32 * HOMING_BUDGET_PERCENT / 100;
        let mut stepped: u32 = 0;
        let mut blank = STALL_BLANK_STEPS;
        let mut next = Instant::now();
        loop {
            if stepped >= budget {
                warn!("Homing gave up after {} steps without a stall", stepped);
                return Err(FaultCause::HomingTimeout);
            }
            if blank > 0 {
                blank -= 1;
            } else if self.diag.is_high() {
                break;
            }

            self.step.set_high();
            cortex_m::asm::delay(STEP_PULSE_CYCLES);
            self.step.set_low();
            stepped += 1;

            if stepped.is_multiple_of(SG_SAMPLE_EVERY_STEPS) {
                match self.tmc.read::<reg::SG_RESULT>().await {
                    Ok(sg) => info!("Homing SG_RESULT: {}", sg.get()),
                    Err(_) => warn!("SG_RESULT read failed during homing"),
                }
                // The read pauses stepping; re-blank so the restart does not
                // false-trigger DIAG.
                blank = STALL_BLANK_STEPS;
                next = Instant::now();
            }

            next += Duration::from_ticks(HOMING_DELAY_TICKS);
            let now = Instant::now();
            if next < now {
                next = now;
            }
            Timer::at(next).await;
        }
        info!("Home stop found after {} steps", stepped);
        Timer::after_millis(50).await;

        self.dir.set_level(away_level());
        Timer::after_ticks(1).await;
        let mut next = Instant::now();
        for _ in 0..BACKOFF_STEPS {
            self.step.set_high();
            cortex_m::asm::delay(STEP_PULSE_CYCLES);
            self.step.set_low();
            next += Duration::from_ticks(HOMING_DELAY_TICKS);
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

    async fn move_to(&mut self, target_steps: i32, mode: MoveMode) -> MoveResult {
        let mut target = target_steps.clamp(0, TRAVEL_STEPS);
        let mut delay_ticks = START_DELAY_TICKS;
        let mut accel_counter: u32 = 0;
        let mut blank = STALL_BLANK_STEPS;
        let mut current_dir: Option<Level> = None;
        let mut next = Instant::now();

        while self.position != target {
            if let Some(command) = SERVO_COMMAND.try_take()
                && command == ServoRudderCommand::Initialize
            {
                return MoveResult::Initialize;
            }
            if mode == MoveMode::Tracking {
                if let Some(setpoint) = SERVO_SETPOINT.try_take() {
                    self.watchdog_deadline = Instant::now() + WATCHDOG_TIMEOUT;
                    CURRENT_SETPOINT.store(setpoint, Ordering::Relaxed);
                    target = setpoint_to_steps(setpoint).clamp(0, TRAVEL_STEPS);
                    continue;
                }
                if Instant::now() >= self.watchdog_deadline {
                    return MoveResult::WatchdogExpired;
                }
            }
            if blank > 0 {
                blank -= 1;
            } else if self.diag.is_high() {
                warn!("Stall detected at position {}", self.position);
                return MoveResult::Stalled;
            }

            let direction = if target > self.position { 1 } else { -1 };
            let dir_level = if direction > 0 {
                away_level()
            } else {
                HOME_DIR_LEVEL
            };
            if current_dir != Some(dir_level) {
                self.dir.set_level(dir_level);
                current_dir = Some(dir_level);
                delay_ticks = START_DELAY_TICKS;
                blank = STALL_BLANK_STEPS;
                Timer::after_ticks(1).await;
                next = Instant::now();
            }

            accel_counter += 1;
            if accel_counter >= ACCEL_EVERY_N_STEPS {
                accel_counter = 0;
                let remaining = (target - self.position).unsigned_abs();
                let decel_steps = (START_DELAY_TICKS - delay_ticks) as u32 * ACCEL_EVERY_N_STEPS;
                if remaining <= decel_steps {
                    if delay_ticks < START_DELAY_TICKS {
                        delay_ticks += 1;
                    }
                } else if delay_ticks > MIN_DELAY_TICKS {
                    delay_ticks -= 1;
                }
            }

            self.step_pulse(direction);

            next += Duration::from_ticks(delay_ticks);
            let now = Instant::now();
            if next < now {
                next = now;
            }
            Timer::at(next).await;
        }
        MoveResult::Reached
    }

    async fn initialize(&mut self) -> State {
        self.set_state(State::Homing, FaultCause::None);
        match self.home().await {
            Ok(()) => {
                self.watchdog_deadline = Instant::now() + WATCHDOG_TIMEOUT;
                CURRENT_SETPOINT.store(steps_to_setpoint(self.position), Ordering::Relaxed);
                self.set_state(State::Operational, FaultCause::None);
                State::Operational
            }
            Err(cause) => {
                self.set_state(State::Fault, cause);
                State::Fault
            }
        }
    }

    /// A stall made the step counter untrustworthy: re-find the home stop so
    /// the servo at least parks (holding) at the failsafe position.
    async fn stall_recover(&mut self) -> State {
        self.set_state(State::Fault, FaultCause::StallDuringMove);
        Timer::after_millis(100).await;
        match self.home().await {
            Ok(()) => info!("Stall recovery: parked at home position"),
            Err(cause) => warn!("Stall recovery failed: {}", cause),
        }
        State::Fault
    }

    async fn enter_failsafe(&mut self) -> State {
        warn!("Setpoint watchdog expired; moving to failsafe position");
        self.set_state(State::FailSafe, FaultCause::None);
        CURRENT_SETPOINT.store(FAILSAFE_SETPOINT, Ordering::Relaxed);
        match self
            .move_to(setpoint_to_steps(FAILSAFE_SETPOINT), MoveMode::Fixed)
            .await
        {
            MoveResult::Reached | MoveResult::WatchdogExpired => State::FailSafe,
            MoveResult::Initialize => self.initialize().await,
            MoveResult::Stalled => self.stall_recover().await,
        }
    }

    /// Uninitialized / FailSafe / Fault: only an Initialize command acts.
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

    async fn operational(&mut self) -> State {
        match select3(
            SERVO_SETPOINT.wait(),
            SERVO_COMMAND.wait(),
            Timer::at(self.watchdog_deadline),
        )
        .await
        {
            Either3::First(setpoint) => {
                self.watchdog_deadline = Instant::now() + WATCHDOG_TIMEOUT;
                CURRENT_SETPOINT.store(setpoint, Ordering::Relaxed);
                match self
                    .move_to(setpoint_to_steps(setpoint), MoveMode::Tracking)
                    .await
                {
                    MoveResult::Reached => State::Operational,
                    MoveResult::Initialize => self.initialize().await,
                    MoveResult::Stalled => self.stall_recover().await,
                    MoveResult::WatchdogExpired => self.enter_failsafe().await,
                }
            }
            Either3::Second(ServoRudderCommand::Initialize) => self.initialize().await,
            Either3::Second(_) => {
                warn!("Unknown servo command ignored");
                State::Operational
            }
            Either3::Third(_) => self.enter_failsafe().await,
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
    };
    let mut state = State::Uninitialized;
    servo.set_state(state, FaultCause::None);

    loop {
        state = match state {
            State::Operational => servo.operational().await,
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
