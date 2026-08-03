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
    peripherals::{self, CAN1, I2C2, UART5},
    usart,
};
use embassy_time::Timer;
use eoi_rust_firmware::app_type::AppType;
use eoi_rust_firmware::can::{can_rx_task, init_can};
use eoi_rust_firmware::clock::clock_config;
use eoi_rust_firmware::flow_sensor;
use eoi_rust_firmware::steering_angle;
use eoi_rust_firmware::temperature::{CAN_ID_TEMPERATURE_RUDDER_CONTROLLER, temperature_task};
use eoi_rust_firmware::{config, cooling_pump, declare_app_type, motor_temperature, servo_rudder};
use {defmt_rtt as _, panic_probe as _};

declare_app_type!(AppType::RudderController);

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    I2C2_EV  => i2c::EventInterruptHandler<I2C2>;
    I2C2_ER  => i2c::ErrorInterruptHandler<I2C2>;
    UART5    => usart::InterruptHandler<UART5>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
    DMA2_CHANNEL1 => dma::InterruptHandler<peripherals::DMA2_CH1>;
    DMA2_CHANNEL2 => dma::InterruptHandler<peripherals::DMA2_CH2>;
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
        config::ConfigStore::new(p.FLASH),
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

    let mut tmc_uart_config = usart::Config::default();
    tmc_uart_config.baudrate = servo_rudder::TMC_BAUD;
    let (tmc_tx, tmc_rx) = usart::Uart::new(
        p.UART5,
        p.PD2,  // RX
        p.PC12, // TX
        p.DMA2_CH1,
        p.DMA2_CH2,
        Irqs,
        tmc_uart_config,
    )
    .unwrap()
    .split();

    let (servo_step, servo_dir, servo_enable, servo_diag) =
        servo_rudder::init(p.PC11, p.PC10, p.PB5, p.PB4);
    spawner.spawn(unwrap!(servo_rudder::servo_control_task(
        servo_step,
        servo_dir,
        servo_enable,
        servo_diag,
        tmc_tx,
        tmc_rx
    )));
    spawner.spawn(unwrap!(servo_rudder::status_task(buffered.writer())));
}
