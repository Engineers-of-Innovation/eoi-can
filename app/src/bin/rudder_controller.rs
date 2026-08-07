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
use eoi_rust_firmware::app_type::AppType;
use eoi_rust_firmware::can::{can_rx_task, init_can};
use eoi_rust_firmware::clock::clock_config;
use eoi_rust_firmware::flow_sensor;
use eoi_rust_firmware::steering_angle;
use eoi_rust_firmware::temperature::{CAN_ID_TEMPERATURE_RUDDER_CONTROLLER, temperature_task};
use eoi_rust_firmware::{cooling_pump, declare_app_type, motor_temperature};
use {defmt_rtt as _, panic_probe as _};

declare_app_type!(AppType::RudderController);

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

    let green_led = Output::new(p.PC1, Level::High, Speed::Low);
    let red_led = Output::new(p.PC2, Level::High, Speed::Low);
    let blue_led = Output::new(p.PC3, Level::High, Speed::Low);
    core::mem::forget(red_led);
    core::mem::forget(blue_led);

    let watchdog = IndependentWatchdog::new(p.IWDG, 4_000_000);
    spawner.spawn(unwrap!(heartbeat_task(green_led, watchdog)));

    let can = Can::new(p.CAN1, p.PB8, p.PB9, Irqs);
    let buffered = init_can(can, p.PB7).await;

    spawner.spawn(unwrap!(can_rx_task(buffered.reader(), MY_APP_TYPE)));

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

    let (cooling_pump_enable, cooling_pump_fault) =
        cooling_pump::init(p.DAC1, p.PA4, p.PA5, p.PA6, p.PA7, p.PC5);

    spawner.spawn(unwrap!(cooling_pump::fault_status_task(
        cooling_pump_fault,
        buffered.writer()
    )));
    spawner.spawn(unwrap!(cooling_pump::enable_control_task(
        cooling_pump_enable
    )));

    let (steering_adc, steering_input, steering_pwm) = steering_angle::init(p.ADC1, p.PB1, p.PB2);
    spawner.spawn(unwrap!(steering_angle::sample_task(
        steering_adc,
        steering_input,
        buffered.writer()
    )));
    spawner.spawn(unwrap!(steering_angle::pwm_task(steering_pwm)));

    let adc2 = flow_sensor::init_adc2(p.ADC2);

    let flow_in = flow_sensor::flow_in_init(p.TIM2, p.PA0, p.PA1);
    spawner.spawn(unwrap!(flow_sensor::flow_in_task(
        flow_in.timer,
        adc2,
        flow_in.ntc,
        buffered.writer()
    )));

    let motor_temp_ntc = motor_temperature::init(p.PA3);
    spawner.spawn(unwrap!(motor_temperature::motor_temp_task(
        adc2,
        motor_temp_ntc,
        buffered.writer()
    )));
}
