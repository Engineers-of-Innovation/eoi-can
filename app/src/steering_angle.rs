use core::sync::atomic::{AtomicI16, Ordering};

use defmt::*;
use embassy_futures::select::{Either, select};
use embassy_stm32::Peri;
use embassy_stm32::adc::{Adc, AdcConfig, Averaging, SampleTime};
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::{ADC1, PB1, PB2};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Ticker, Timer};

use crate::config::{
    CAPTURED_ALL, CAPTURED_CENTER, CAPTURED_LEFT, CAPTURED_RIGHT, ConfigStore, SteeringCal,
};

pub const CAN_ID_STEERING_ANGLE: StandardId = unsafe { StandardId::new_unchecked(0x213) };
pub const CAN_ID_STEERING_CAL_CMD: StandardId = unsafe { StandardId::new_unchecked(0x214) };
pub const CAN_ID_STEERING_CAL_ACK: StandardId = unsafe { StandardId::new_unchecked(0x218) };

/// Reported position is normalized against the calibrated travel:
/// full left = -1000, centre = 0, full right = +1000 (0.1 % steps).
pub const POSITION_FULL_SCALE: i16 = 1000;

const ADC_MAX: u16 = 4095;

/// Reject a calibration whose half-travel is narrower than this many ADC
/// codes; below that the normalized output is mostly ADC noise.
const MIN_HALF_SPAN: i32 = 100;

/// Readings averaged when capturing a calibration point, spread at
/// `OVERSAMPLE_PERIOD_MS` — 200 ms of data for a one-off manual operation.
const CAPTURE_SAMPLES: u32 = 40;

// Status bits, reported in byte 4 of CAN_ID_STEERING_ANGLE.
/// Calibration is present and plausible; the reported position is meaningful.
pub const STATUS_CAL_VALID: u8 = 1 << 0;
/// No calibration has been stored yet, or all stored records are corrupt.
pub const STATUS_CAL_MISSING: u8 = 1 << 1;
/// A calibration is stored but is incomplete or implausible.
pub const STATUS_CAL_INVALID: u8 = 1 << 2;
/// The raw reading is outside the calibrated travel; position is clamped.
pub const STATUS_OUT_OF_RANGE: u8 = 1 << 3;
/// The last write to persistent storage failed; sticky until the next success.
pub const STATUS_STORAGE_ERROR: u8 = 1 << 4;

// Calibration command results, reported in byte 1 of CAN_ID_STEERING_CAL_ACK.
const CAL_RESULT_OK: u8 = 0;
const CAL_RESULT_STORAGE_ERROR: u8 = 1;
/// Stored, but the set as a whole is still not usable — see the status byte.
const CAL_RESULT_INCOMPLETE: u8 = 2;

// 10 Hz CAN broadcast rate.
const SAMPLE_PERIOD_MS: u64 = 100;

// Noise rejection has two layers, because either alone leaves a gap.
//
// The ADC hardware averages 16 conversions per read (set in `init`). That
// suppresses white noise by sqrt(16) = 4x, but the burst spans only ~52 us, so
// it does nothing for interference slower than roughly 20 kHz.
//
// On top of that, reads are spread evenly across the reporting window and
// box-averaged. Averaging over exactly one window puts nulls at 10 Hz and every
// harmonic, which is what rejects periodic interference from the stepper driver
// and cooling pump. Combined white-noise rejection is sqrt(320) = ~18x.
//
// Cost is 20 reads x 16 conversions = 320 conversions per report. At 260 ADC
// cycles each and an 80 MHz ADC clock that is ~1.04 ms per 100 ms, so ~1 % duty.
// Group delay is half the window, 50 ms, inherent to averaging over it.
const OVERSAMPLE_PERIOD_MS: u64 = 5;
const OVERSAMPLES_PER_REPORT: u32 = (SAMPLE_PERIOD_MS / OVERSAMPLE_PERIOD_MS) as u32;

// 1 Hz software PWM, 1 % duty resolution.
const PWM_PERIOD_MS: u64 = 1000;
const PWM_TICK_MS: u64 = 10;
const PWM_STEPS: u64 = PWM_PERIOD_MS / PWM_TICK_MS;

/// Latest normalized position, shared between the sample task and the PWM
/// task. Held at 0 (centre) whenever the calibration is not usable.
static LATEST_POSITION: AtomicI16 = AtomicI16::new(0);

/// Calibration command from the CAN receive task.
pub static STEERING_CAL_COMMAND: Signal<CriticalSectionRawMutex, CalCommand> = Signal::new();

/// Calibration is captured live: move the steering to the position, then send
/// the matching command. No values travel over the bus.
#[derive(Clone, Copy, PartialEq, Eq, Format)]
pub enum CalCommand {
    CaptureLeft,
    CaptureCenter,
    CaptureRight,
    /// Discard the calibration and fall back to the uncalibrated safe state.
    Clear,
}

impl CalCommand {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::CaptureLeft),
            0x02 => Some(Self::CaptureCenter),
            0x03 => Some(Self::CaptureRight),
            0x04 => Some(Self::Clear),
            _ => None,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::CaptureLeft => 0x01,
            Self::CaptureCenter => 0x02,
            Self::CaptureRight => 0x03,
            Self::Clear => 0x04,
        }
    }
}

pub fn init(
    adc1: Peri<'static, ADC1>,
    input_pin: Peri<'static, PB1>,
    pwm_pin: Peri<'static, PB2>,
) -> (Adc<'static, ADC1>, Peri<'static, PB1>, Output<'static>) {
    // Hardware oversampling: each read runs 16 conversions and returns their
    // sum right-shifted by 4, so the result stays on the same 12-bit scale and
    // calibration codes stored by earlier firmware remain comparable.
    // Keep this ratio in step with the arithmetic in the comment above.
    let adc = Adc::new_with_config(
        adc1,
        AdcConfig {
            averaging: Some(Averaging::Samples16),
            ..Default::default()
        },
    );
    let pwm = Output::new(pwm_pin, Level::Low, Speed::Low);
    (adc, input_pin, pwm)
}

/// Signed distance from centre to each endpoint, or `None` when the
/// calibration cannot be used to map a reading.
///
/// Rejects an incomplete set, out-of-range codes, a centre that does not sit
/// strictly between the endpoints, and a travel too narrow to resolve.
fn spans(cal: &SteeringCal) -> Option<(i32, i32)> {
    if cal.captured != CAPTURED_ALL {
        return None;
    }
    if cal.left > ADC_MAX || cal.center > ADC_MAX || cal.right > ADC_MAX {
        return None;
    }
    let left = cal.left as i32 - cal.center as i32;
    let right = cal.right as i32 - cal.center as i32;
    if left.abs() < MIN_HALF_SPAN || right.abs() < MIN_HALF_SPAN {
        return None;
    }
    // Opposite signs mean the centre lies between the endpoints. Equal signs
    // mean both endpoints are on the same side of it, which is unusable.
    // Either wiring polarity is accepted.
    if left.signum() == right.signum() {
        return None;
    }
    Some((left, right))
}

/// Map a raw ADC code onto the normalized position, piecewise-linear through
/// the calibrated centre. Also reports whether the reading fell outside the
/// calibrated travel and had to be clamped.
fn raw_to_position(raw: u16, cal: &SteeringCal, left_span: i32, right_span: i32) -> (i16, bool) {
    let offset = raw as i32 - cal.center as i32;
    // `offset` and the span for its side always share a sign, so the
    // magnitude below is non-negative.
    let toward_right = (offset >= 0) == (right_span > 0);
    let span = if toward_right { right_span } else { left_span };

    let full_scale = POSITION_FULL_SCALE as i32;
    let magnitude = offset * full_scale / span;
    let position = if toward_right { magnitude } else { -magnitude };

    (
        position.clamp(-full_scale, full_scale) as i16,
        magnitude > full_scale,
    )
}

fn read_raw(adc: &mut Adc<'static, ADC1>, input: &mut Peri<'static, PB1>) -> u16 {
    adc.blocking_read(input, SampleTime::CYCLES247_5)
}

/// Average readings spread over time so a captured endpoint is not set by a
/// noisy moment. Spread rather than back-to-back for the same reason the
/// reporting path spreads its reads: a burst only averages noise faster than
/// the burst itself.
async fn capture_raw(adc: &mut Adc<'static, ADC1>, input: &mut Peri<'static, PB1>) -> u16 {
    let mut sum = 0u32;
    for _ in 0..CAPTURE_SAMPLES {
        sum += read_raw(adc, input) as u32;
        Timer::after(Duration::from_millis(OVERSAMPLE_PERIOD_MS)).await;
    }
    ((sum + CAPTURE_SAMPLES / 2) / CAPTURE_SAMPLES) as u16
}

/// Bits 0..2 are mutually exclusive: valid, or stored-but-unusable, or nothing
/// stored at all. `any_captured` distinguishes the latter two — a cleared or
/// never-written block reads back with no endpoints captured.
fn status_bits(any_captured: bool, spans_ok: bool, out_of_range: bool, storage_error: bool) -> u8 {
    let mut status = 0;
    if spans_ok {
        status |= STATUS_CAL_VALID;
        if out_of_range {
            status |= STATUS_OUT_OF_RANGE;
        }
    } else if any_captured {
        status |= STATUS_CAL_INVALID;
    } else {
        status |= STATUS_CAL_MISSING;
    }
    if storage_error {
        status |= STATUS_STORAGE_ERROR;
    }
    status
}

#[embassy_executor::task]
pub async fn sample_task(
    mut adc: Adc<'static, ADC1>,
    mut input: Peri<'static, PB1>,
    mut store: ConfigStore,
    mut can_tx: BufferedCanSender,
) {
    let mut cal = store.load().unwrap_or_default();
    let mut storage_error = false;
    match spans(&cal) {
        Some(_) => info!(
            "Steering calibration loaded: left {} centre {} right {}",
            cal.left, cal.center, cal.right
        ),
        None => warn!(
            "Steering calibration unusable (captured {:#04x}), reporting 0",
            cal.captured
        ),
    }

    let mut ticker = Ticker::every(Duration::from_millis(OVERSAMPLE_PERIOD_MS));
    let debug_every_n_reports = 10;
    let mut report_count: u32 = 0;
    // Accumulator for the reads making up the current reporting window.
    let mut sum: u32 = 0;
    let mut count: u32 = 0;

    loop {
        match select(ticker.next(), STEERING_CAL_COMMAND.wait()).await {
            Either::First(_) => {
                sum += read_raw(&mut adc, &mut input) as u32;
                count += 1;
                if count < OVERSAMPLES_PER_REPORT {
                    continue;
                }
                // Round rather than truncate; at most 20 * 4095 so no overflow.
                let raw = ((sum + count / 2) / count) as u16;
                sum = 0;
                count = 0;

                // Without a usable calibration the position stays at centre
                // and the error is reported in the status byte.
                let cal_spans = spans(&cal);
                let (position, out_of_range) = match cal_spans {
                    Some((left, right)) => raw_to_position(raw, &cal, left, right),
                    None => (0, false),
                };
                let status = status_bits(
                    cal.captured != 0,
                    cal_spans.is_some(),
                    out_of_range,
                    storage_error,
                );
                LATEST_POSITION.store(position, Ordering::Relaxed);

                report_count += 1;
                if report_count.is_multiple_of(debug_every_n_reports) {
                    info!(
                        "Steering position: {} (raw {}, status {:#04x})",
                        position, raw, status
                    );
                    report_count = 0;
                }

                let position_le = position.to_le_bytes();
                let raw_le = raw.to_le_bytes();
                let frame = Frame::new_data(
                    CAN_ID_STEERING_ANGLE,
                    &[position_le[0], position_le[1], raw_le[0], raw_le[1], status],
                )
                .unwrap();
                if let Err(e) = can_tx.try_write(frame) {
                    warn!("Steering angle CAN tx error: {:?}", e);
                }
            }

            Either::Second(command) => {
                let captured = match command {
                    CalCommand::Clear => {
                        cal = SteeringCal::default();
                        0
                    }
                    CalCommand::CaptureLeft => {
                        cal.left = capture_raw(&mut adc, &mut input).await;
                        cal.captured |= CAPTURED_LEFT;
                        cal.left
                    }
                    CalCommand::CaptureCenter => {
                        cal.center = capture_raw(&mut adc, &mut input).await;
                        cal.captured |= CAPTURED_CENTER;
                        cal.center
                    }
                    CalCommand::CaptureRight => {
                        cal.right = capture_raw(&mut adc, &mut input).await;
                        cal.captured |= CAPTURED_RIGHT;
                        cal.right
                    }
                };

                let spans_ok = spans(&cal).is_some();
                let result = match store.store(&cal) {
                    Ok(()) => {
                        storage_error = false;
                        info!(
                            "Steering {:?}: raw {} stored (captured {:#04x}, usable {})",
                            command, captured, cal.captured, spans_ok
                        );
                        if spans_ok {
                            CAL_RESULT_OK
                        } else {
                            CAL_RESULT_INCOMPLETE
                        }
                    }
                    Err(e) => {
                        storage_error = true;
                        error!("Steering calibration store failed: {:?}", e);
                        CAL_RESULT_STORAGE_ERROR
                    }
                };

                // The new calibration takes effect on the next sample tick;
                // park at centre in the meantime if it is not usable.
                if !spans_ok {
                    LATEST_POSITION.store(0, Ordering::Relaxed);
                }

                let captured_le = captured.to_le_bytes();
                let status = status_bits(cal.captured != 0, spans_ok, false, storage_error);
                let frame = Frame::new_data(
                    CAN_ID_STEERING_CAL_ACK,
                    &[
                        command.as_u8(),
                        result,
                        captured_le[0],
                        captured_le[1],
                        cal.captured,
                        status,
                    ],
                )
                .unwrap();
                if let Err(e) = can_tx.try_write(frame) {
                    warn!("Steering calibration ack CAN tx error: {:?}", e);
                }

                // Capturing and storing took far longer than one tick, so the
                // ticker owes a backlog it would otherwise fire off immediately.
                // Resync it and drop the half-filled window, so the first report
                // after calibration is a properly spread average rather than a
                // burst — which is exactly the value the operator is watching.
                ticker.reset();
                sum = 0;
                count = 0;
            }
        }
    }
}

#[embassy_executor::task]
pub async fn pwm_task(mut pwm: Output<'static>) {
    loop {
        // Map -POSITION_FULL_SCALE..+POSITION_FULL_SCALE to 0..PWM_STEPS, so
        // centre — including the uncalibrated safe state — is 50 % duty.
        let position = LATEST_POSITION.load(Ordering::Relaxed) as i64;
        let full_scale = POSITION_FULL_SCALE as i64;
        let high_steps =
            ((position + full_scale) as u64 * PWM_STEPS / (2 * full_scale) as u64).min(PWM_STEPS);
        let low_steps = PWM_STEPS - high_steps;

        if high_steps > 0 {
            pwm.set_high();
            Timer::after(Duration::from_millis(high_steps * PWM_TICK_MS)).await;
        }
        if low_steps > 0 {
            pwm.set_low();
            Timer::after(Duration::from_millis(low_steps * PWM_TICK_MS)).await;
        }
    }
}
