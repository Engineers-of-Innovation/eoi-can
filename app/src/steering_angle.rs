use core::sync::atomic::{AtomicI16, Ordering};
use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::adc::{Adc, SampleTime};
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::{ADC1, PB1, PB2};
use embassy_time::{Duration, Ticker, Timer};

pub const CAN_ID_STEERING_ANGLE: StandardId = unsafe { StandardId::new_unchecked(0x213) };

// Linear mapping: ADC code 0 -> -180°, 4095 -> +180°.
const ADC_MAX: i32 = 4095;
const ANGLE_SPAN_DEG: i32 = 360;
const ANGLE_OFFSET_DEG: i32 = -180;

// 10 Hz CAN broadcast / sample rate.
const SAMPLE_PERIOD_MS: u64 = 100;

// 1 Hz software PWM, 1 % duty resolution.
const PWM_PERIOD_MS: u64 = 1000;
const PWM_TICK_MS: u64 = 10;
const PWM_STEPS: u64 = PWM_PERIOD_MS / PWM_TICK_MS;

// Latest sampled angle, shared between the sample task and the PWM task.
static LATEST_ANGLE_DEG: AtomicI16 = AtomicI16::new(0);

pub fn init(
    adc1: Peri<'static, ADC1>,
    input_pin: Peri<'static, PB1>,
    pwm_pin: Peri<'static, PB2>,
) -> (Adc<'static, ADC1>, Peri<'static, PB1>, Output<'static>) {
    let adc = Adc::new(adc1);
    let pwm = Output::new(pwm_pin, Level::Low, Speed::Low);
    (adc, input_pin, pwm)
}

fn raw_to_angle_deg(raw: u16) -> i16 {
    let raw = raw as i32;
    let angle = (raw * ANGLE_SPAN_DEG / ADC_MAX) + ANGLE_OFFSET_DEG;
    angle.clamp(-180, 180) as i16
}

#[embassy_executor::task]
pub async fn sample_task(
    mut adc: Adc<'static, ADC1>,
    mut input: Peri<'static, PB1>,
    mut can_tx: BufferedCanSender,
) {
    let mut ticker = Ticker::every(Duration::from_millis(SAMPLE_PERIOD_MS));
    loop {
        ticker.next().await;

        let raw = adc.blocking_read(&mut input, SampleTime::CYCLES247_5);
        let angle = raw_to_angle_deg(raw);
        LATEST_ANGLE_DEG.store(angle, Ordering::Relaxed);

        let angle_le = angle.to_le_bytes();
        let raw_le = raw.to_le_bytes();
        let frame = Frame::new_data(
            CAN_ID_STEERING_ANGLE,
            &[angle_le[0], angle_le[1], raw_le[0], raw_le[1]],
        )
        .unwrap();
        if let Err(e) = can_tx.try_write(frame) {
            warn!("Steering angle CAN tx error: {:?}", e);
        }
    }
}

#[embassy_executor::task]
pub async fn pwm_task(mut pwm: Output<'static>) {
    loop {
        let angle = LATEST_ANGLE_DEG.load(Ordering::Relaxed) as i32;
        // Map -180..+180 to 0..PWM_STEPS.
        let high_steps =
            ((angle - ANGLE_OFFSET_DEG) as u64 * PWM_STEPS / ANGLE_SPAN_DEG as u64).min(PWM_STEPS);
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
