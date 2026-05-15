#![no_std]
#![no_main]

mod bootloader;
mod flash;

#[cfg(all(feature = "rudder-controller", feature = "height-sensor-controller"))]
compile_error!(
    "exactly one of the `rudder-controller` / `height-sensor-controller` features must be enabled, not both"
);

#[cfg(not(any(feature = "rudder-controller", feature = "height-sensor-controller")))]
compile_error!(
    "one of the `rudder-controller` / `height-sensor-controller` features must be enabled"
);

#[cfg(feature = "rudder-controller")]
pub const EXPECTED_APP_TYPE: eoi_boot_api::header::AppType =
    eoi_boot_api::header::AppType::RudderController;
#[cfg(feature = "height-sensor-controller")]
pub const EXPECTED_APP_TYPE: eoi_boot_api::header::AppType =
    eoi_boot_api::header::AppType::HeightSensorController;

use core::cell::RefCell;

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::can::{
    Can, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler, TxInterruptHandler,
};
use embassy_stm32::can::{RxBuf, TxBuf};
use embassy_stm32::flash::Flash;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::CAN1;
use embassy_stm32::rcc::{
    AHBPrescaler, APBPrescaler, Hse, HseMode, LsConfig, LseConfig, LseDrive, LseMode, Pll, PllMul,
    PllPreDiv, PllRDiv, PllSource, RtcClockSource, Sysclk,
};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, can};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, Ticker};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
});

type BlockingFlash = Flash<'static, embassy_stm32::flash::Blocking>;

static TX_BUF: StaticCell<TxBuf<8>> = StaticCell::new();
static RX_BUF: StaticCell<RxBuf<8>> = StaticCell::new();
static CAN: StaticCell<Can<'static>> = StaticCell::new();
static FLASH: StaticCell<Mutex<NoopRawMutex, RefCell<BlockingFlash>>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("EoI Bootloader starting");

    let p = embassy_stm32::init(clock_config());

    // Status LED - PC1
    let status_led = Output::new(p.PC2, Level::High, Speed::Low);
    spawner.spawn(unwrap!(heartbeat_task(status_led)));

    // CAN bus - PB8 (RX), PB9 (TX), PB7 (standby)
    let standby_out = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(standby_out);

    let can = CAN.init(Can::new(p.CAN1, p.PB8, p.PB9, Irqs));
    can.modify_filters()
        .enable_bank(0, can::Fifo::Fifo0, can::filter::Mask32::accept_all());
    can.modify_config().set_bitrate(1_000_000);
    can.enable().await;
    let buffered = can.buffered(TX_BUF.init(TxBuf::new()), RX_BUF.init(RxBuf::new()));

    // Flash peripheral
    let flash = Flash::new_blocking(p.FLASH);
    let flash = FLASH.init(Mutex::new(RefCell::new(flash)));

    // Initialize flash layout and bootloader
    let flash_layout = flash::FlashLayout::new(flash);
    let bl = bootloader::Bootloader::init(flash_layout, buffered, p.IWDG);

    info!("Starting bootloader task");
    spawner.spawn(unwrap!(bootloader_task(bl)));
}

#[embassy_executor::task]
async fn bootloader_task(
    mut bl: bootloader::Bootloader<'static, NoopRawMutex, BlockingFlash>,
) -> ! {
    bl.run().await
}

#[embassy_executor::task]
async fn heartbeat_task(mut led: Output<'static>) -> ! {
    let mut ticker = Ticker::every(Duration::from_secs(1));
    loop {
        // Double flash pattern to distinguish from application
        for _ in 0..2 {
            led.set_low();
            embassy_time::Timer::after(Duration::from_millis(100)).await;
            led.set_high();
            embassy_time::Timer::after(Duration::from_millis(100)).await;
        }
        ticker.next().await;
    }
}

fn clock_config() -> embassy_stm32::Config {
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(16_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV1,
        mul: PllMul::MUL10,
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2),
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.apb2_pre = APBPrescaler::DIV1;
    config.rcc.ls = LsConfig {
        rtc: RtcClockSource::LSE,
        lsi: false,
        lse: Some(LseConfig {
            frequency: Hertz(32_768),
            mode: LseMode::Oscillator(LseDrive::MediumHigh),
        }),
    };
    config
}
