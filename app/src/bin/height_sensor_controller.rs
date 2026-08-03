#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{
    bind_interrupts,
    can::{Can, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler, TxInterruptHandler},
    dma,
    gpio::{Input, Level, Output, Pull, Speed},
    i2c,
    peripherals::{self, CAN1, I2C2, USART2, USART3},
    usart,
};
use embassy_time::Timer;
use eoi_rust_firmware::app_type::AppType;
use eoi_rust_firmware::can::{can_rx_task, init_can};
use eoi_rust_firmware::clock::clock_config;
use eoi_rust_firmware::declare_app_type;
use eoi_rust_firmware::height_sensor::{
    CAN_ID_HEIGHT_SENSOR_FRONT_LEFT, CAN_ID_HEIGHT_SENSOR_FRONT_RIGHT, height_sensor_task,
    height_sensor_timer_task,
};
use eoi_rust_firmware::temperature::{CAN_ID_TEMPERATURE_HEIGHT_SENSORS, temperature_task};
use {defmt_rtt as _, panic_probe as _};

declare_app_type!(AppType::HeightSensorController);

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    I2C2_EV  => i2c::EventInterruptHandler<I2C2>;
    I2C2_ER  => i2c::ErrorInterruptHandler<I2C2>;
    USART2   => usart::InterruptHandler<USART2>;
    USART3   => usart::InterruptHandler<USART3>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
    DMA1_CHANNEL3 => dma::InterruptHandler<peripherals::DMA1_CH3>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
    DMA1_CHANNEL6 => dma::InterruptHandler<peripherals::DMA1_CH6>;
    DMA1_CHANNEL7 => dma::InterruptHandler<peripherals::DMA1_CH7>;
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
    info!("Height Sensor Controller");

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
        CAN_ID_TEMPERATURE_HEIGHT_SENSORS,
        buffered.writer()
    )));

    let mut uart_config = usart::Config::default();
    uart_config.baudrate = 9600;
    uart_config.parity = usart::Parity::ParityNone;
    uart_config.stop_bits = usart::StopBits::STOP2;
    uart_config.data_bits = usart::DataBits::DataBits8;

    // HeightSensorFrontLeft — USART2
    let uart = usart::Uart::new_with_de(
        p.USART2,
        p.PA3,      // RX
        p.PA2,      // TX
        p.PA1,      // DE (RS-485 direction)
        p.DMA1_CH7, // TX DMA
        p.DMA1_CH6, // RX DMA
        Irqs,
        uart_config,
    )
    .unwrap();
    let height_detect = Input::new(p.PA0, Pull::Down);
    spawner.spawn(unwrap!(height_sensor_task(
        uart,
        height_detect,
        CAN_ID_HEIGHT_SENSOR_FRONT_LEFT,
        buffered.writer(),
        0,
    )));

    // HeightSensorFrontRight — USART3
    let uart3 = usart::Uart::new_with_de(
        p.USART3,
        p.PC5,      // RX
        p.PC4,      // TX
        p.PB1,      // DE (RS-485 direction)
        p.DMA1_CH2, // TX DMA
        p.DMA1_CH3, // RX DMA
        Irqs,
        uart_config,
    )
    .unwrap();
    let height_detect3 = Input::new(p.PB2, Pull::Down);
    spawner.spawn(unwrap!(height_sensor_task(
        uart3,
        height_detect3,
        CAN_ID_HEIGHT_SENSOR_FRONT_RIGHT,
        buffered.writer(),
        1,
    )));

    // Alternating 16 Hz timer: FL, FR, FL, FR, ... (8 Hz each, never started together)
    spawner.spawn(unwrap!(height_sensor_timer_task()));
}
