//! Motor temperature from a 10 kΩ NTC on the rudder controller's steering
//! potentiometer input.
//!
//! This is a port of the standalone `can-motor-temperature` node (an STM32G491
//! on CANable 2.5 hardware) onto rudder-controller hardware. The CAN frame and
//! the conversion chain are deliberately identical, so `eoi-can-decoder` and
//! everything downstream of it needs no change — see `MotorNtc` there and the
//! `0x219` row in `CAN_MESSAGES.md`.
//!
//! # Hardware
//!
//! ```text
//!   PB2 ──[ 10k ]── PB1 ──[ 47R ]──[ 10k NTC ]── GND
//! pot_feedback      │  pot_analog_in
//!  (bias, switched) └─ ADC1_IN16 + any filter capacitor to GND
//! ```
//!
//! The pull-up is fed from PB2 rather than from the 3V3 rail, which buys two
//! things beyond saving the divider's 165 µA:
//!
//! - **The measurement is ratiometric.** The divider is fed from VDD through
//!   PB2 and the ADC reference is VDDA — the same rail — so the supply voltage
//!   cancels out of the ratio. No VREFINT correction, and supply ripple from
//!   the motor drive does not appear in the reading.
//! - **Self-heating is negligible.** The NTC dissipates ~0.27 mW while biased,
//!   and it is biased for only ~215 ms per second.
//!
//! When the bias is off, PB2 is driven **low**, not left floating: both ends of
//! the divider then sit at ground, so no current flows and the sense node
//! cannot be pumped around by capacitive coupling from the motor cables.
//!
//! # Filtering
//!
//! Four stages, sized for a sensor sitting next to motor cables:
//!
//! | Stage | What |
//! |---|---|
//! | analog | whatever filter capacitor is fitted on the sense node |
//! | ADC hardware oversampler | 256 accumulations per read, 16-bit result, no CPU cost |
//! | trimmed mean | 16 reads spread over 64 ms, sorted, highest 4 and lowest 4 discarded |
//! | IIR + slew limit | first-order IIR (~4 s), then at most 5.0 °C change per update |
//!
//! 4096 hardware samples per second, of which 2048 can be arbitrarily corrupted
//! without moving the result. The trimming is the part that matters here: an
//! interference burst landing inside one block is thrown away outright rather
//! than averaged in, which a plain mean cannot do.

use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::adc::vals::{OversamplingRatio, OversamplingShift, Rovsm, Trovs};
use embassy_stm32::adc::{Adc, AdcConfig, SampleTime};
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::{ADC1, PB1, PB2};
use embassy_time::{Duration, Ticker, Timer};
use libm::logf;

pub const CAN_ID_MOTOR_NTC: StandardId = unsafe { StandardId::new_unchecked(0x219) };

// Status bits, reported in byte 2 of the CAN frame. Same encoding as
// `eSensorStatus` in the standalone node's `Source/ntc.h`.
/// Divider tap sits at the bias rail: the NTC is disconnected.
pub const STATUS_SENSOR_OPEN: u8 = 0x01;
/// Divider tap sits at ground: the NTC is shorted.
pub const STATUS_SENSOR_SHORT: u8 = 0x02;
/// Conversion succeeded but is outside the plausibility window; value clamped.
pub const STATUS_OUT_OF_RANGE: u8 = 0x04;
/// The IIR filter has not yet been fed enough updates.
pub const STATUS_SETTLING: u8 = 0x08;
/// The ADC delivered no burst this cycle. Kept for wire compatibility with the
/// standalone node; this port reads the ADC synchronously with no DMA to fail,
/// so nothing sets it.
pub const STATUS_ACQUISITION_ERROR: u8 = 0x10;
/// The previous frame could not be queued for transmission.
pub const STATUS_CAN_TX_FAILED: u8 = 0x20;

/// Transmitted when no valid temperature is available. Chosen so a receiver
/// cannot mistake a fault for a plausible reading even from the first two bytes
/// alone.
pub const INVALID_DECIDEG: i16 = i16::MIN; // 0x8000

// ---- Divider, in ohm.
/// Pull-up from `pot_feedback` (PB2) to the sense node.
const R_PULLUP_OHMS: f32 = 10_000.0;
/// On-resistance of the PB2 push-pull driver, in series with the pull-up.
/// Worth about 0.1 °C at room temperature.
const BIAS_DRIVER_OHMS: f32 = 30.0;
/// Board resistor between the sense node and the NTC, in the low leg of the
/// divider. Subtracted from the measured leg resistance. Set this to 0 if the
/// 47 Ω turns out to sit between the connector and the MCU pin instead: an
/// ADC input draws no DC, so a resistor there does not divide anything.
const R_SERIES_LOW_OHMS: f32 = 47.0;
const R_PULLUP_TOTAL_OHMS: f32 = R_PULLUP_OHMS + BIAS_DRIVER_OHMS;

// ---- NTC. B25/100 = 3988 for the fitted part — a 25..100 °C fit, not the more
// common B25/85, which suits a motor sensor because the model is most accurate
// exactly where this sensor operates. A single B value is only exact at its two
// fit points, so expect a degree or two of error near -40 °C.
const NTC_R25_OHMS: f32 = 10_000.0;
const NTC_BETA_K: f32 = 3988.0;
const KELVIN_AT_0C: f32 = 273.15;
const KELVIN_AT_25C: f32 = 298.15;

// ---- Plausibility. The ratios correspond to roughly -51 °C and +181 °C, so
// the reported window fits comfortably inside them.
const TEMP_MIN_DECIDEG: i32 = -400;
const TEMP_MAX_DECIDEG: i32 = 1500;
const RATIO_OPEN: f32 = 0.990;
const RATIO_SHORT: f32 = 0.010;

// ---- Timing. One second, from the top:
//
//     t = 0 ms     bias on
//     t = 150 ms   ADC burst starts (15 tau of capacitor settling)
//     t = 150..214 16 oversampled reads spread across the burst
//     t = 214 ms   trimmed mean, IIR, convert, transmit, bias off
//
/// Broadcast interval.
const UPDATE_PERIOD_MS: u64 = 1000;
/// How long the divider is biased before the ADC starts.
///
/// This must cover the settling time of whatever filter capacitor is on the
/// sense node, and the figure that matters is the worst case: tau tends to
/// `R_PULLUP_OHMS * C` when the NTC is cold and its resistance is high, *not*
/// the parallel combination with a room-temperature NTC. 10 k and 1 µF is
/// tau = 10 ms, so this is 15 tau for the largest capacitor likely to be fitted.
/// The cost is only the divider's bias current, not CPU time.
const BIAS_SETTLE_MS: u64 = 150;

// ---- Oversampling and filtering.
/// Hardware oversampling ratio, accumulated and shifted by the ADC itself.
const OVERSAMPLE_RATIO: u32 = 256;
/// Right shift applied to the accumulator. 4095 * 256 / 2^4 = 65520, so the
/// result keeps four bits of resolution over the raw 12-bit conversion and
/// still fits in the 16-bit data register.
const OVERSAMPLE_SHIFT: u32 = 4;
const ADC_FULL_SCALE: u32 = 4095 * OVERSAMPLE_RATIO / (1 << OVERSAMPLE_SHIFT);

/// Oversampled reads collected per update, from which the trimmed mean is taken.
const BLOCK_COUNT: usize = 16;
/// Reads discarded from each end after sorting. 4 keeps the middle 8 of 16.
const TRIM_COUNT: usize = 4;
const KEPT_COUNT: u32 = (BLOCK_COUNT - 2 * TRIM_COUNT) as u32;
/// Spacing between reads. Each read is 2 x 256 conversions at 260 ADC cycles
/// and 80 MHz, about 1.7 ms, so the burst spans ~64 ms with the core idle in
/// between. Spread rather than back-to-back for the same reason the steering
/// angle spreads its reads: a burst only averages noise faster than the burst.
const BLOCK_PERIOD_MS: u64 = 4;

/// First-order IIR across updates: `y += (x - y) >> shift`. The time constant is
/// about `2^shift - 1` updates, so 2 is ~3 s at the 1 Hz update rate.
const IIR_SHIFT: u32 = 2;
/// Maximum accepted change per update, in 0.1 °C. A genuine motor temperature
/// cannot move this fast, so this is the last line of defence against an
/// outlier that survived every other stage.
const MAX_SLEW_DECIDEG: i32 = 50;

/// Configure ADC1 for 256x hardware oversampling and claim the two pot pins.
///
/// The bias output starts low, which is the de-energised state of the divider.
pub fn init(
    adc1: Peri<'static, ADC1>,
    analog_pin: Peri<'static, PB1>,
    bias_pin: Peri<'static, PB2>,
) -> (Adc<'static, ADC1>, Peri<'static, PB1>, Output<'static>) {
    // Set the oversampler explicitly rather than through `averaging`, which
    // always shifts the accumulator all the way back to 12 bits. Here the shift
    // is deliberately four bits short of that, which is what keeps the extra
    // resolution the 256 accumulations bought.
    let adc = Adc::new_with_config(
        adc1,
        AdcConfig {
            oversampling_ratio: Some(OversamplingRatio::RATIO256),
            oversampling_shift: Some(OversamplingShift::SHIFT4),
            // No injected conversions exist here, so the regular/injected
            // interaction mode is irrelevant; the `true` is what enables the
            // oversampler at all.
            oversampling_mode: Some((Rovsm::CONTINUED, Trovs::AUTOMATIC, true)),
            ..Default::default()
        },
    );
    let bias = Output::new(bias_pin, Level::Low, Speed::Low);
    (adc, analog_pin, bias)
}

/// Collect one burst and return its trimmed mean, in Q8.
async fn trimmed_mean_q8(adc: &mut Adc<'static, ADC1>, input: &mut Peri<'static, PB1>) -> u32 {
    let mut samples = [0u16; BLOCK_COUNT];
    let mut ticker = Ticker::every(Duration::from_millis(BLOCK_PERIOD_MS));
    for (i, sample) in samples.iter_mut().enumerate() {
        if i != 0 {
            ticker.next().await;
        }
        *sample = adc.blocking_read(input, SampleTime::CYCLES247_5);
    }

    // BLOCK_COUNT is small, so an insertion sort is the right tool.
    for i in 1..BLOCK_COUNT {
        let v = samples[i];
        let mut j = i;
        while j > 0 && samples[j - 1] > v {
            samples[j] = samples[j - 1];
            j -= 1;
        }
        samples[j] = v;
    }

    let sum: u32 = samples[TRIM_COUNT..BLOCK_COUNT - TRIM_COUNT]
        .iter()
        .map(|&v| v as u32)
        .sum();
    // At most 8 * 65520, so the Q8 shift cannot overflow.
    (sum << 8) / KEPT_COUNT
}

/// First-order IIR across updates, carried in Q8 so small changes are not lost
/// to truncation.
struct Iir {
    state_q8: u32,
    primed: bool,
    updates: u32,
}

impl Iir {
    const fn new() -> Self {
        Self {
            state_q8: 0,
            primed: false,
            updates: 0,
        }
    }

    /// Feed one trimmed mean and return the filtered code.
    fn update(&mut self, mean_q8: u32) -> u32 {
        if self.primed {
            let error = mean_q8 as i32 - self.state_q8 as i32;
            self.state_q8 = (self.state_q8 as i32 + (error >> IIR_SHIFT)) as u32;
        } else {
            // Start at the first reading rather than ramping up from zero.
            self.state_q8 = mean_q8;
            self.primed = true;
        }
        if self.updates < (1 << IIR_SHIFT) {
            self.updates += 1;
        }
        (self.state_q8 + 128) >> 8
    }

    fn is_settled(&self) -> bool {
        self.primed && self.updates >= (1 << IIR_SHIFT)
    }
}

/// Resistance to temperature, by the beta model `R = R25 * exp(B * (1/T - 1/T25))`.
///
/// `None` for anything the model cannot represent, which includes a
/// non-positive resistance: the logarithm takes those to -inf or NaN and both
/// fall out of the checks below.
///
/// Floating point is used deliberately: this runs once per second and a
/// logarithm costs tens of microseconds. Accuracy is worth far more.
fn resistance_to_celsius(r_ntc: f32) -> Option<f32> {
    let inv_t = 1.0 / KELVIN_AT_25C + logf(r_ntc / NTC_R25_OHMS) / NTC_BETA_K;
    if inv_t <= 0.0 {
        return None;
    }
    let celsius = 1.0 / inv_t - KELVIN_AT_0C;
    celsius.is_finite().then_some(celsius)
}

/// Convert a filtered ADC code to 0.1 °C, applying the slew limit across calls.
///
/// Returns [`INVALID_DECIDEG`] and sets the matching status bit on a detected
/// sensor fault.
fn code_to_decidegrees(code: u32, status: &mut u8, last: &mut Option<i16>) -> i16 {
    let ratio = code as f32 / ADC_FULL_SCALE as f32;

    // Which rail means "open" follows from the topology: the NTC is the low leg,
    // so losing it lets the sense node float up to the bias rail.
    if ratio >= RATIO_OPEN {
        *status |= STATUS_SENSOR_OPEN;
        *last = None;
        return INVALID_DECIDEG;
    }
    if ratio <= RATIO_SHORT {
        *status |= STATUS_SENSOR_SHORT;
        *last = None;
        return INVALID_DECIDEG;
    }

    // Divider -> NTC resistance. `ratio` measures the whole low leg, so the
    // board resistor in series with the NTC comes back off.
    let r_ntc = R_PULLUP_TOTAL_OHMS * ratio / (1.0 - ratio) - R_SERIES_LOW_OHMS;
    let Some(celsius) = resistance_to_celsius(r_ntc) else {
        *status |= STATUS_OUT_OF_RANGE;
        *last = None;
        return INVALID_DECIDEG;
    };

    // Scale to 0.1 °C, rounding to nearest.
    let scaled = celsius * 10.0;
    let mut decideg = if scaled >= 0.0 {
        (scaled + 0.5) as i32
    } else {
        (scaled - 0.5) as i32
    };

    // Plausibility window. Report the clamped value plus a flag rather than a
    // wild one.
    if decideg < TEMP_MIN_DECIDEG {
        decideg = TEMP_MIN_DECIDEG;
        *status |= STATUS_OUT_OF_RANGE;
    } else if decideg > TEMP_MAX_DECIDEG {
        decideg = TEMP_MAX_DECIDEG;
        *status |= STATUS_OUT_OF_RANGE;
    }

    if let Some(previous) = *last {
        decideg = decideg.clamp(
            previous as i32 - MAX_SLEW_DECIDEG,
            previous as i32 + MAX_SLEW_DECIDEG,
        );
    }

    let decideg = decideg as i16;
    *last = Some(decideg);
    decideg
}

/// Bias the divider, read it, convert it, broadcast it. Once a second, forever.
#[embassy_executor::task]
pub async fn motor_ntc_task(
    mut adc: Adc<'static, ADC1>,
    mut input: Peri<'static, PB1>,
    mut bias: Output<'static>,
    mut can_tx: BufferedCanSender,
) {
    let mut iir = Iir::new();
    let mut last_decideg: Option<i16> = None;
    let mut tx_failed_last = false;
    let mut frame_counter: u8 = 0;

    let mut ticker = Ticker::every(Duration::from_millis(UPDATE_PERIOD_MS));
    loop {
        bias.set_high();
        Timer::after(Duration::from_millis(BIAS_SETTLE_MS)).await;

        let mean_q8 = trimmed_mean_q8(&mut adc, &mut input).await;
        bias.set_low();

        let mut status = if tx_failed_last {
            STATUS_CAN_TX_FAILED
        } else {
            0
        };
        // Read before this update is folded in, which is where the standalone
        // node checks it: the flag clears on the first frame *after* the IIR has
        // had its full 2^IIR_SHIFT updates, not on the frame that completes them.
        if !iir.is_settled() {
            status |= STATUS_SETTLING;
        }
        let code = iir.update(mean_q8);
        let decideg = code_to_decidegrees(code, &mut status, &mut last_decideg);

        if decideg == INVALID_DECIDEG {
            warn!(
                "Motor NTC: no reading (code {}, status {:#04x})",
                code, status
            );
        } else {
            info!(
                "Motor NTC: {} °C (code {}, status {:#04x})",
                decideg as f32 / 10.0,
                code,
                status
            );
        }

        let decideg_le = decideg.to_le_bytes();
        let frame = Frame::new_data(
            CAN_ID_MOTOR_NTC,
            &[decideg_le[0], decideg_le[1], status, frame_counter],
        )
        .unwrap();
        // `try_write` only reports a full TX buffer, not a frame the bus never
        // acknowledged, so 0x20 is a weaker signal here than on the standalone
        // node — where it means the previous transmission went unacknowledged.
        tx_failed_last = match can_tx.try_write(frame) {
            Ok(()) => false,
            Err(e) => {
                warn!("Motor NTC CAN tx error: {:?}", e);
                true
            }
        };
        frame_counter = frame_counter.wrapping_add(1);

        ticker.next().await;
    }
}
