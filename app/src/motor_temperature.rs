use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::adc::SampleTime;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::peripherals::PA3;
use embassy_time::{Duration, Ticker};

use crate::flow_sensor::{Adc2Mutex, NtcParams, ntc_raw_to_centidegrees};

pub const CAN_ID_MOTOR_TEMP: StandardId = unsafe { StandardId::new_unchecked(0x217) };

// NTC voltage divider: 10 kΩ to VREF on top, NTC to GND on bottom.
// NTC: 10 kΩ at 25 °C, B = 3950 K.
const MOTOR_NTC: NtcParams = NtcParams {
    top_ohms: 10_000.0,
    r0_ohms: 10_000.0,
    t0_k: 298.15,
    b_k: 3950.0,
};

const TICK_PERIOD_MS: u64 = 1000;

pub fn init(ntc_pin: Peri<'static, PA3>) -> Peri<'static, PA3> {
    ntc_pin
}

#[embassy_executor::task]
pub async fn motor_temp_task(
    adc: &'static Adc2Mutex,
    mut ntc: Peri<'static, PA3>,
    mut can_tx: BufferedCanSender,
) {
    let mut ticker = Ticker::every(Duration::from_millis(TICK_PERIOD_MS));
    loop {
        ticker.next().await;

        let raw_adc = {
            let mut guard = adc.lock().await;
            guard.blocking_read(&mut ntc, SampleTime::CYCLES247_5)
        };
        let cdeg = ntc_raw_to_centidegrees(raw_adc, &MOTOR_NTC);

        info!(
            "Motor temp: {} °C (raw ADC {})",
            cdeg as f32 / 100.0,
            raw_adc
        );

        let cdeg_le = cdeg.to_le_bytes();
        let raw_le = raw_adc.to_le_bytes();
        let frame = Frame::new_data(
            CAN_ID_MOTOR_TEMP,
            &[cdeg_le[0], cdeg_le[1], raw_le[0], raw_le[1]],
        )
        .unwrap();
        if let Err(e) = can_tx.try_write(frame) {
            warn!("Motor temp CAN tx error: {:?}", e);
        }
    }
}
