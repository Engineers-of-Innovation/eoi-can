//! Motor temperature sensor on rudder-controller hardware.
//!
//! A 10 kΩ NTC on `pot_analog_in` (PB1), biased through a 10 kΩ pull-up from
//! `pot_feedback` (PB2), read once a second and broadcast on CAN `0x219` in
//! 0.1 °C. Wiring, filtering and frame layout live in
//! [`eoi_firmware::motor_ntc`]; this binary is only the board bring-up.
//!
//! It is the same node as the standalone `can-motor-temperature` firmware, on
//! different hardware — deliberately identical on the wire, so nothing
//! downstream has to know which one is fitted.
//!
//! No bootloader: built without the `bootloader` feature the image owns all of
//! flash from 0x08000000 and is flashed with a debug probe. It therefore
//! declares no app type and answers no bootloader commands, so `eoi-flash-tool
//! scan` will not see it.

#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{
    bind_interrupts,
    can::{Can, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler, TxInterruptHandler},
    gpio::{Level, Output, Speed},
    peripherals::CAN1,
};
use embassy_time::Timer;
use eoi_firmware::can::init_can;
use eoi_firmware::clock::clock_config;
use eoi_firmware::motor_ntc;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
});

#[embassy_executor::task]
async fn heartbeat_task(
    mut output: Output<'static>,
    mut watchdog: IndependentWatchdog<'static, embassy_stm32::peripherals::IWDG>,
) {
    watchdog.unleash();
    loop {
        watchdog.pet();
        output.toggle();
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    info!("Motor NTC Sensor");
    eoi_firmware::build_info::log();

    let green_led = Output::new(p.PC1, Level::High, Speed::Low);
    core::mem::forget(Output::new(p.PC2, Level::High, Speed::Low));
    core::mem::forget(Output::new(p.PC3, Level::High, Speed::Low));

    // This is rudder-controller hardware, so the stepper driver and the cooling
    // pump driver may still be wired up even though nothing here drives them.
    // Park both in their off state instead of leaving the pins as inputs and
    // relying on whatever the boards pull them to. Stepper enable is
    // active-low; the pump's is active-high.
    core::mem::forget(Output::new(p.PB5, Level::High, Speed::Low));
    core::mem::forget(Output::new(p.PA6, Level::Low, Speed::Low));

    let watchdog = IndependentWatchdog::new(p.IWDG, 4_000_000);
    spawner.spawn(unwrap!(heartbeat_task(green_led, watchdog)));

    let can = Can::new(p.CAN1, p.PB8, p.PB9, Irqs);
    // Transmit only. The accept-all filter costs an interrupt per bus frame and
    // the buffered receiver drops what nothing reads, which is what we want:
    // this node has nothing to say back.
    let buffered = init_can(can, p.PB7).await;

    let (adc, analog_in, bias) = motor_ntc::init(p.ADC1, p.PB1, p.PB2);
    spawner.spawn(unwrap!(motor_ntc::motor_ntc_task(
        adc,
        analog_in,
        bias,
        buffered.writer()
    )));
}
