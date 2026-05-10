use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::dac::{DacChannel, Value};
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::mode::Blocking;
use embassy_stm32::peripherals::{DAC1, PA4, PA5, PA6, PA7, PC5};
use embassy_time::Timer;

// Cooling-pump motor driver: I_max = VREF / 0.66 V/A.
// VDDA is the DAC reference (no VREFBUF configured in clock_config()).
const CURRENT_LIMIT_AMPS: f32 = 1.0;
const VDDA_VOLTS: f32 = 3.3;
const DRIVER_VREF_PER_AMP: f32 = 0.66;

const CURRENT_LIMIT_DAC_CODE: u16 =
    (CURRENT_LIMIT_AMPS * DRIVER_VREF_PER_AMP / VDDA_VOLTS * 4095.0 + 0.5) as u16;

pub const CAN_ID_COOLING_PUMP_STATUS: StandardId = unsafe { StandardId::new_unchecked(0x212) };

pub fn init(
    dac1: Peri<'static, DAC1>,
    vref_pin: Peri<'static, PA4>,
    sleep_pin: Peri<'static, PA5>,
    enable_pin: Peri<'static, PA6>,
    direction_pin: Peri<'static, PA7>,
    fault_pin: Peri<'static, PC5>,
) -> Input<'static> {
    // Drive motor driver into wake / enabled / forward state.
    core::mem::forget(Output::new(sleep_pin, Level::High, Speed::Low));
    core::mem::forget(Output::new(enable_pin, Level::High, Speed::Low));
    core::mem::forget(Output::new(direction_pin, Level::Low, Speed::Low));

    let mut dac = DacChannel::new_blocking(dac1, vref_pin);
    dac.set(Value::Bit12Right(CURRENT_LIMIT_DAC_CODE));
    core::mem::forget(dac); // Prevent the DAC from being deinitialized, which would disable the output.

    // Fault is open-drain active-low on the driver, so pull up internally.
    let fault = Input::new(fault_pin, Pull::Up);

    fault
}

#[embassy_executor::task]
pub async fn fault_status_task(fault: Input<'static>, mut can_tx: BufferedCanSender) {
    loop {
        let value: u8 = if fault.is_high() { 1 } else { 0 };
        let frame = Frame::new_data(CAN_ID_COOLING_PUMP_STATUS, &[value]).unwrap();
        if let Err(e) = can_tx.try_write(frame) {
            warn!("Cooling pump status CAN tx error: {:?}", e);
        }
        Timer::after_secs(1).await;
    }
}
