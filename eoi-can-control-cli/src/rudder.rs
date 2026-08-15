use std::io::Write as _;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail, ensure};
use clap::Subcommand;
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, execute};
use embedded_can::{Frame as _, StandardId};
use eoi_can_decoder::{EoiCanData, RudderControllerData, ServoData, ServoState, ServoStatus};
use socketcan::CanFrame;
use socketcan::tokio::CanSocket;
use tracing::{info, warn};

pub const CAN_ID_SETPOINT: u16 = 0x010;
pub const CAN_ID_COMMAND: u16 = 0x021;
pub const SETPOINT_MIN: u16 = 1000;
pub const SETPOINT_MAX: u16 = 2000;
const COMMAND_INITIALIZE: u8 = 0x00;

/// The controller broadcasts status every 100 ms; silence this long means it is absent.
const STATUS_SILENCE: Duration = Duration::from_secs(3);
/// How long a state may persist before we trust it as the controller's real reaction
/// (frames sent just before our command may still show the pre-command state).
const STATE_SETTLE_GRACE: Duration = Duration::from_secs(2);

#[derive(Subcommand, Debug)]
pub enum RudderCommand {
    /// Hold the rudder at a setpoint, re-sending it to feed the firmware's 2 s watchdog
    Set {
        /// Servo setpoint: 1000 (home/failsafe end) to 2000 (far end)
        #[arg(value_parser = clap::value_parser!(u16).range(SETPOINT_MIN as i64..=SETPOINT_MAX as i64))]
        setpoint: u16,

        /// Send a single frame and exit; the firmware fail-safes ~2 s later
        #[arg(long)]
        once: bool,

        /// Re-send frequency in Hz
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=100))]
        rate: u8,

        /// Send Initialize first and wait for homing to finish before starting
        #[arg(long = "init")]
        init_first: bool,
    },
    /// Send Initialize ((re)start homing) and wait for the result
    Init {
        /// Fire the command without waiting for the homing result
        #[arg(long)]
        no_wait: bool,

        /// Seconds to wait for the controller to reach Operational
        #[arg(long, default_value_t = 90.0)]
        timeout: f64,
    },
    /// Print decoded rudder status frames (0x020) until Ctrl-C
    Status {
        /// Print the first status frame and exit
        #[arg(long)]
        once: bool,
    },
    /// Sweep the setpoint back and forth between two bounds for bench testing
    Sweep {
        /// Sweep start setpoint
        #[arg(long = "from", default_value_t = SETPOINT_MIN, value_parser = clap::value_parser!(u16).range(SETPOINT_MIN as i64..=SETPOINT_MAX as i64))]
        start: u16,

        /// Sweep end setpoint
        #[arg(long = "to", default_value_t = SETPOINT_MAX, value_parser = clap::value_parser!(u16).range(SETPOINT_MIN as i64..=SETPOINT_MAX as i64))]
        end: u16,

        /// Seconds for one full there-and-back cycle
        #[arg(long, default_value_t = 20.0)]
        period: f64,

        /// Send frequency in Hz
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=100))]
        rate: u8,

        /// Number of cycles; runs until Ctrl-C when omitted
        #[arg(long)]
        cycles: Option<u32>,

        /// Send Initialize first and wait for homing to finish before starting
        #[arg(long = "init")]
        init_first: bool,
    },
    /// Drive the rudder with the keyboard (←/→ nudge, Home/End jump, i = init, q = quit)
    Interactive {
        /// Setpoint change per arrow-key press
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=1000))]
        step: u16,

        /// Send Initialize first and wait for homing to finish before starting
        #[arg(long = "init")]
        init_first: bool,
    },
}

/// Default wait for `--init` preludes, matching the `init` subcommand's default.
const INIT_DEFAULT_TIMEOUT: f64 = 90.0;

pub async fn run(command: RudderCommand, interface: &str) -> anyhow::Result<()> {
    let socket =
        CanSocket::open(interface).with_context(|| format!("opening CAN interface {interface}"))?;

    match command {
        RudderCommand::Set {
            setpoint,
            once,
            rate,
            init_first,
        } => {
            if init_first {
                init(&socket, false, INIT_DEFAULT_TIMEOUT).await?;
            }
            set(&socket, setpoint, once, rate).await
        }
        RudderCommand::Init { no_wait, timeout } => init(&socket, no_wait, timeout).await,
        RudderCommand::Status { once } => status(&socket, once).await,
        RudderCommand::Sweep {
            start,
            end,
            period,
            rate,
            cycles,
            init_first,
        } => {
            if init_first {
                init(&socket, false, INIT_DEFAULT_TIMEOUT).await?;
            }
            sweep(&socket, start, end, period, rate, cycles).await
        }
        RudderCommand::Interactive { step, init_first } => {
            if init_first {
                init(&socket, false, INIT_DEFAULT_TIMEOUT).await?;
            }
            crate::interactive::run(&socket, step).await
        }
    }
}

pub fn setpoint_frame(setpoint: u16) -> CanFrame {
    CanFrame::new(
        StandardId::new(CAN_ID_SETPOINT).unwrap(),
        &setpoint.to_le_bytes(),
    )
    .unwrap()
}

pub fn initialize_frame() -> CanFrame {
    CanFrame::new(
        StandardId::new(CAN_ID_COMMAND).unwrap(),
        &[COMMAND_INITIALIZE],
    )
    .unwrap()
}

/// Decode a received frame down to a rudder servo status, ignoring everything else on the bus.
pub fn decode_status(frame: &socketcan::CanFrame) -> Option<ServoStatus> {
    let socketcan::CanFrame::Data(frame) = frame else {
        return None;
    };
    let frame = eoi_can_decoder::can_frame::CanFrame::from_encoded(frame.id(), frame.data());
    match eoi_can_decoder::parse_eoi_can_data(&frame) {
        Some(EoiCanData::RudderController(RudderControllerData::Servo(ServoData::Status(
            status,
        )))) => Some(status),
        _ => None,
    }
}

/// Follows the 0x020 status stream and decides when a reported state is the
/// controller's real reaction to our command rather than a stale broadcast.
struct StatusTracker {
    started: Instant,
    last_frame_at: Option<Instant>,
    state_since: Instant,
    last: Option<ServoStatus>,
    saw_non_fault: bool,
}

impl StatusTracker {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_frame_at: None,
            state_since: now,
            last: None,
            saw_non_fault: false,
        }
    }

    /// Record a status frame; returns true when (state, fault cause) changed.
    fn record(&mut self, status: ServoStatus) -> bool {
        self.last_frame_at = Some(Instant::now());
        if status.state != ServoState::Fault {
            self.saw_non_fault = true;
        }
        let changed = !matches!(
            &self.last,
            Some(prev) if prev.state == status.state && prev.fault_cause == status.fault_cause
        );
        if changed {
            self.state_since = Instant::now();
        }
        self.last = Some(status);
        changed
    }

    fn last(&self) -> Option<&ServoStatus> {
        self.last.as_ref()
    }

    fn silent_for(&self) -> Duration {
        self.last_frame_at.unwrap_or(self.started).elapsed()
    }

    /// A Fault we should trust: either we watched the controller transition into it,
    /// or Fault is all we have seen for longer than the settle grace (the controller
    /// re-faulted faster than one status period, or ignored the command).
    fn fault_confirmed(&self) -> Option<&ServoStatus> {
        let last = self.last.as_ref()?;
        (last.state == ServoState::Fault
            && (self.saw_non_fault || self.state_since.elapsed() > STATE_SETTLE_GRACE))
            .then_some(last)
    }

    /// A state in which the controller ignores setpoints, persisting past the grace.
    fn locked_state_timeout(&self) -> Option<&ServoStatus> {
        let last = self.last.as_ref()?;
        (matches!(last.state, ServoState::Uninitialized | ServoState::FailSafe)
            && self.state_since.elapsed() > STATE_SETTLE_GRACE)
            .then_some(last)
    }
}

pub fn draw_status_line(setpoint: u16, status: Option<&ServoStatus>) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        cursor::MoveToColumn(0),
        Clear(ClearType::CurrentLine)
    )?;
    match status {
        Some(status) => write!(
            stdout,
            "setpoint: {setpoint} | state: {:?}  actual: {}  fault: {:?}",
            status.state, status.actual_position, status.fault_cause
        )?,
        None => write!(stdout, "setpoint: {setpoint} | no status received yet")?,
    }
    stdout.flush()?;
    Ok(())
}

/// Clear the live status line so a full log line can be printed cleanly.
fn clear_status_line() {
    let mut stdout = std::io::stdout();
    let _ = execute!(
        stdout,
        cursor::MoveToColumn(0),
        Clear(ClearType::CurrentLine)
    );
}

/// Terminal conditions while we are actively commanding setpoints.
fn hold_failure(tracker: &StatusTracker) -> Option<anyhow::Error> {
    if let Some(status) = tracker.fault_confirmed() {
        return Some(anyhow!(
            "controller faulted: {:?} — run `rudder init` to recover",
            status.fault_cause
        ));
    }
    if let Some(status) = tracker.locked_state_timeout() {
        return Some(anyhow!(
            "controller is {:?} and ignoring setpoints — run `rudder init` first",
            status.state
        ));
    }
    None
}

/// One step of a setpoint-holding loop.
enum HoldStep {
    Send(u16),
    /// Send this final setpoint, then stop.
    Finish(u16),
}

/// Drive setpoints at `rate` Hz while following the controller's status:
/// a live line shows the incoming state, Fault/locked states abort with an error,
/// and silence is warned about once (bench use without a controller stays possible).
async fn hold_and_monitor(
    socket: &CanSocket,
    rate: u8,
    mut next: impl FnMut(f64) -> HoldStep,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let mut tracker = StatusTracker::new();
    let mut warned_silent = false;
    let mut current = SETPOINT_MIN;

    let mut tx = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(rate)));
    let mut housekeeping = tokio::time::interval(Duration::from_secs(1));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = tx.tick() => {
                match next(started.elapsed().as_secs_f64()) {
                    HoldStep::Send(setpoint) => {
                        current = setpoint;
                        socket.write_frame(setpoint_frame(setpoint)).await?;
                        draw_status_line(current, tracker.last())?;
                    }
                    HoldStep::Finish(setpoint) => {
                        socket.write_frame(setpoint_frame(setpoint)).await?;
                        clear_status_line();
                        info!("done, parked at {setpoint}");
                        return Ok(());
                    }
                }
            }
            frame = socket.read_frame() => {
                if let Some(status) = decode_status(&frame?) {
                    tracker.record(status);
                    draw_status_line(current, tracker.last())?;
                    if let Some(error) = hold_failure(&tracker) {
                        clear_status_line();
                        return Err(error);
                    }
                }
            }
            _ = housekeeping.tick() => {
                if let Some(error) = hold_failure(&tracker) {
                    clear_status_line();
                    return Err(error);
                }
                if !warned_silent && tracker.silent_for() > STATUS_SILENCE {
                    warned_silent = true;
                    clear_status_line();
                    warn!(
                        "no status frames from the rudder controller — is it powered and on the bus? (still transmitting)"
                    );
                }
            }
            _ = &mut ctrl_c => {
                clear_status_line();
                info!("stopping; the firmware watchdog will park the rudder at {SETPOINT_MIN}");
                return Ok(());
            }
        }
    }
}

async fn set(socket: &CanSocket, setpoint: u16, once: bool, rate: u8) -> anyhow::Result<()> {
    if once {
        socket.write_frame(setpoint_frame(setpoint)).await?;
        info!("sent single setpoint {setpoint}; the firmware fail-safes ~2 s from now");
        return report_next_status(socket).await;
    }

    info!("holding setpoint {setpoint} at {rate} Hz, Ctrl-C to stop");
    hold_and_monitor(socket, rate, |_| HoldStep::Send(setpoint)).await
}

/// After a one-shot send, show the controller's next status so the result is visible.
async fn report_next_status(socket: &CanSocket) -> anyhow::Result<()> {
    let next_status = async {
        loop {
            let frame = socket.read_frame().await?;
            if let Some(status) = decode_status(&frame) {
                return anyhow::Ok(status);
            }
        }
    };
    match tokio::time::timeout(Duration::from_secs(1), next_status).await {
        Ok(status) => {
            let status = status?;
            info!(
                "controller: state {:?}, setpoint {}, actual {}, fault {:?}",
                status.state, status.setpoint, status.actual_position, status.fault_cause
            );
            ensure!(
                status.state != ServoState::Fault,
                "controller is in Fault ({:?}) — run `rudder init` to recover",
                status.fault_cause
            );
            Ok(())
        }
        Err(_) => {
            warn!("no status frame within 1 s — is the controller powered and on the bus?");
            Ok(())
        }
    }
}

async fn init(socket: &CanSocket, no_wait: bool, timeout: f64) -> anyhow::Result<()> {
    socket.write_frame(initialize_frame()).await?;
    if no_wait {
        info!("sent Initialize (not waiting for the result)");
        return Ok(());
    }

    info!("sent Initialize; waiting for homing to finish (timeout {timeout} s, Ctrl-C to abort)");
    let mut tracker = StatusTracker::new();
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let mut housekeeping = tokio::time::interval(Duration::from_secs(1));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            frame = socket.read_frame() => {
                let Some(status) = decode_status(&frame?) else { continue };
                if tracker.record(status) {
                    let status = tracker.last().unwrap();
                    info!(
                        "t={:.1}s  state: {:?} (fault cause: {:?})",
                        tracker.started.elapsed().as_secs_f64(),
                        status.state,
                        status.fault_cause
                    );
                }
                let status = tracker.last().unwrap();
                if status.state == ServoState::Operational {
                    info!(
                        "homing complete after {:.1} s, actual position {}",
                        tracker.started.elapsed().as_secs_f64(),
                        status.actual_position
                    );
                    return Ok(());
                }
                if let Some(error) = init_failure(&tracker) {
                    return Err(error);
                }
            }
            _ = housekeeping.tick() => {
                ensure!(
                    tracker.silent_for() < STATUS_SILENCE,
                    "no status frames from the rudder controller — is it powered and on the bus?"
                );
                if let Some(error) = init_failure(&tracker) {
                    return Err(error);
                }
                if Instant::now() >= deadline {
                    match tracker.last() {
                        Some(status) => bail!(
                            "controller did not reach Operational within {timeout} s (still {:?})",
                            status.state
                        ),
                        None => bail!("controller did not reach Operational within {timeout} s"),
                    }
                }
            }
            _ = &mut ctrl_c => bail!("aborted while waiting for the homing result"),
        }
    }
}

/// Terminal failure conditions while waiting for a homing result.
fn init_failure(tracker: &StatusTracker) -> Option<anyhow::Error> {
    if let Some(status) = tracker.fault_confirmed() {
        return Some(anyhow!("homing failed: fault {:?}", status.fault_cause));
    }
    if let Some(status) = tracker.locked_state_timeout() {
        return Some(anyhow!(
            "controller is still {:?} — it does not seem to have acted on Initialize",
            status.state
        ));
    }
    None
}

async fn status(socket: &CanSocket, once: bool) -> anyhow::Result<()> {
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let frame = tokio::select! {
            frame = socket.read_frame() => frame?,
            _ = &mut ctrl_c => return Ok(()),
        };
        if let Some(status) = decode_status(&frame) {
            println!(
                "state: {:?}  setpoint: {}  actual: {}  fault: {:?}",
                status.state, status.setpoint, status.actual_position, status.fault_cause
            );
            if once {
                return Ok(());
            }
        }
    }
}

/// Triangle wave between `start` and `end` with the given cycle period, in setpoint units.
fn triangle_setpoint(start: u16, end: u16, period: f64, elapsed: f64) -> u16 {
    let phase = (elapsed % period) / period;
    let fraction = if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    };
    (f64::from(start) + (f64::from(end) - f64::from(start)) * fraction).round() as u16
}

async fn sweep(
    socket: &CanSocket,
    start: u16,
    end: u16,
    period: f64,
    rate: u8,
    cycles: Option<u32>,
) -> anyhow::Result<()> {
    ensure!(period > 0.0, "--period must be positive");

    match cycles {
        Some(cycles) => info!("sweeping {start} <-> {end}, {period} s/cycle, {cycles} cycle(s)"),
        None => info!("sweeping {start} <-> {end}, {period} s/cycle, Ctrl-C to stop"),
    }

    hold_and_monitor(socket, rate, |elapsed| {
        if let Some(cycles) = cycles
            && elapsed >= period * f64::from(cycles)
        {
            // Finish on the sweep start so the last commanded position is deterministic.
            return HoldStep::Finish(start);
        }
        HoldStep::Send(triangle_setpoint(start, end, period, elapsed))
    })
    .await
}

#[cfg(test)]
mod tests {
    use eoi_can_decoder::ServoFaultCause;

    use super::*;

    fn status(state: ServoState, fault_cause: ServoFaultCause) -> ServoStatus {
        ServoStatus {
            state,
            setpoint: 1500,
            actual_position: 1500,
            fault_cause,
        }
    }

    #[test]
    fn setpoint_frame_is_little_endian() {
        // Matches the documented manual commands: cansend can0 010#E803 / 010#D007.
        assert_eq!(setpoint_frame(1000).data(), &[0xE8, 0x03]);
        assert_eq!(setpoint_frame(2000).data(), &[0xD0, 0x07]);
        assert_eq!(
            setpoint_frame(1000).id(),
            embedded_can::Id::Standard(StandardId::new(0x010).unwrap())
        );
    }

    #[test]
    fn initialize_frame_is_single_zero_byte() {
        assert_eq!(initialize_frame().data(), &[0x00]);
        assert_eq!(
            initialize_frame().id(),
            embedded_can::Id::Standard(StandardId::new(0x021).unwrap())
        );
    }

    #[test]
    fn triangle_hits_endpoints_and_midpoint() {
        assert_eq!(triangle_setpoint(1000, 2000, 10.0, 0.0), 1000);
        assert_eq!(triangle_setpoint(1000, 2000, 10.0, 2.5), 1500);
        assert_eq!(triangle_setpoint(1000, 2000, 10.0, 5.0), 2000);
        assert_eq!(triangle_setpoint(1000, 2000, 10.0, 7.5), 1500);
        // Wraps around to the next cycle.
        assert_eq!(triangle_setpoint(1000, 2000, 10.0, 10.0), 1000);
        assert_eq!(triangle_setpoint(1000, 2000, 10.0, 12.5), 1500);
    }

    #[test]
    fn triangle_supports_descending_sweeps() {
        assert_eq!(triangle_setpoint(2000, 1200, 4.0, 0.0), 2000);
        assert_eq!(triangle_setpoint(2000, 1200, 4.0, 2.0), 1200);
        assert_eq!(triangle_setpoint(2000, 1200, 4.0, 3.0), 1600);
    }

    #[test]
    fn record_reports_state_and_fault_transitions() {
        let mut tracker = StatusTracker::new();
        assert!(tracker.record(status(ServoState::Homing, ServoFaultCause::None)));
        assert!(!tracker.record(status(ServoState::Homing, ServoFaultCause::None)));
        assert!(tracker.record(status(ServoState::Fault, ServoFaultCause::HomingTimeout)));
    }

    #[test]
    fn fault_after_observed_transition_is_confirmed_immediately() {
        let mut tracker = StatusTracker::new();
        tracker.record(status(ServoState::Homing, ServoFaultCause::None));
        tracker.record(status(
            ServoState::Fault,
            ServoFaultCause::DriverNoUartResponse,
        ));
        let fault = tracker.fault_confirmed().expect("fault should be terminal");
        assert_eq!(fault.fault_cause, ServoFaultCause::DriverNoUartResponse);
    }

    #[test]
    fn lone_fault_needs_the_settle_grace() {
        let mut tracker = StatusTracker::new();
        tracker.record(status(ServoState::Fault, ServoFaultCause::StallDuringMove));
        // Could still be a stale broadcast from before our command.
        assert!(tracker.fault_confirmed().is_none());
        tracker.state_since = Instant::now() - STATE_SETTLE_GRACE - Duration::from_millis(1);
        assert!(tracker.fault_confirmed().is_some());
    }

    #[test]
    fn locked_states_time_out_but_active_states_do_not() {
        let mut tracker = StatusTracker::new();
        tracker.record(status(ServoState::FailSafe, ServoFaultCause::None));
        assert!(tracker.locked_state_timeout().is_none());
        tracker.state_since = Instant::now() - STATE_SETTLE_GRACE - Duration::from_millis(1);
        assert!(tracker.locked_state_timeout().is_some());

        let mut tracker = StatusTracker::new();
        tracker.record(status(ServoState::Homing, ServoFaultCause::None));
        tracker.state_since = Instant::now() - STATE_SETTLE_GRACE - Duration::from_millis(1);
        assert!(tracker.locked_state_timeout().is_none());
        assert!(tracker.fault_confirmed().is_none());
    }
}
