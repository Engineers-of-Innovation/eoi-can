#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{
    bind_interrupts,
    can::{Can, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler, TxInterruptHandler},
    dma,
    gpio::{Level, Output, Speed},
    i2c,
    peripherals::{self, CAN1, I2C2},
};
use embassy_time::Timer;
use eoi_rust_firmware::can::{can_rx_task, init_can};
use eoi_rust_firmware::clock::clock_config;
use eoi_rust_firmware::cooling_pump;
use eoi_rust_firmware::temperature::{CAN_ID_TEMPERATURE_RUDDER_CONTROLLER, temperature_task};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    I2C2_EV  => i2c::EventInterruptHandler<I2C2>;
    I2C2_ER  => i2c::ErrorInterruptHandler<I2C2>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
});

#[embassy_executor::task]
async fn heartbeat_task(
    mut output: embassy_stm32::gpio::Output<'static>,
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
    info!("Rudder Controller");

    let (_cooling_pump_current_ref, cooling_pump_fault) =
        cooling_pump::init(p.DAC1, p.PA4, p.PA5, p.PA6, p.PA7, p.PC5);

    let status_led = Output::new(p.PC1, Level::High, Speed::Low);
    let watchdog = IndependentWatchdog::new(p.IWDG, 4_000_000);
    spawner.spawn(unwrap!(heartbeat_task(status_led, watchdog)));

    let can = Can::new(p.CAN1, p.PB8, p.PB9, Irqs);
    let buffered = init_can(can, p.PB7).await;

    spawner.spawn(unwrap!(can_rx_task(buffered.reader())));

    let i2c = i2c::I2c::new(
        p.I2C2,
        p.PB10, // SCL
        p.PB11, // SDA
        p.DMA1_CH4,
        p.DMA1_CH5,
        Irqs,
        Default::default(),
    );

    spawner.spawn(unwrap!(temperature_task(
        i2c,
        CAN_ID_TEMPERATURE_RUDDER_CONTROLLER,
        buffered.writer()
    )));

    spawner.spawn(unwrap!(cooling_pump::fault_status_task(
        cooling_pump_fault,
        buffered.writer()
    )));
}
