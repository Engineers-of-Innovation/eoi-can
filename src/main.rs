#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::can::{Frame, StandardId};
use embassy_stm32::rcc::{
    AHBPrescaler, APBPrescaler, Hse, HseMode, LsConfig, LseConfig, LseDrive, LseMode, Pll, PllMul,
    PllPreDiv, PllRDiv, PllSource, RtcClockSource, Sysclk,
};
use embassy_stm32::time::Hertz;
use embassy_stm32::{
    bind_interrupts,
    can::{
        Can, CanRx, CanTx, Fifo, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler,
        TxInterruptHandler, filter::Mask32,
    },
    gpio::{Level, Output, Speed},
    i2c::{self, Master},
    mode::Async,
    peripherals::{CAN1, I2C2},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer, with_timeout};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

fn clock_config() -> embassy_stm32::Config {
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(16_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,  // 16 MHz HSE
        prediv: PllPreDiv::DIV1, // /1 → 16 MHz
        mul: PllMul::MUL10,      // ×10 → 160 MHz VCO
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2), // /2 → 80 MHz SYSCLK
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

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    I2C2_EV  => i2c::EventInterruptHandler<I2C2>;
    I2C2_ER  => i2c::ErrorInterruptHandler<I2C2>;
});

static CAN_TX: StaticCell<Mutex<NoopRawMutex, CanTx<'static>>> = StaticCell::new();

#[embassy_executor::task]
async fn heartbeat_task(mut output: embassy_stm32::gpio::Output<'static>) {
    loop {
        output.toggle();
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn can_rx_task(mut rx: CanRx<'static>) {
    loop {
        match rx.read().await {
            Ok(envelope) => info!("CAN rx: {:02x}", envelope.frame.data()),
            Err(e) => trace!("CAN rx error: {}", e),
        }
    }
}

#[embassy_executor::task]
async fn temperature_task(
    mut i2c: i2c::I2c<'static, Async, Master>,
    can_tx: &'static Mutex<NoopRawMutex, CanTx<'static>>,
) {
    const ADDR: u8 = 0x3F;
    const REG_SOFT_RESET: u8 = 0x0C;
    const REG_CTRL: u8 = 0x04;
    const REG_DATA_T_L: u8 = 0x06;
    // CTRL: BDU=1 (bit6) | AVG=00 (bit4) | IF_ADD_INC=1 (bit3) | FREERUN=1 (bit2)
    const CTRL_VAL: u8 = 0x4C;

    // Software reset
    i2c.write(ADDR, &[REG_SOFT_RESET, 0x02]).await.ok(); // SWRESET = 1
    i2c.write(ADDR, &[REG_SOFT_RESET, 0x00]).await.ok(); // SWRESET = 0
    Timer::after_millis(5).await;

    // Configure continuous measurement mode
    i2c.write(ADDR, &[REG_CTRL, CTRL_VAL]).await.ok();

    loop {
        let mut buf = [0u8; 2];
        match with_timeout(
            Duration::from_millis(100),
            i2c.write_read(ADDR, &[REG_DATA_T_L], &mut buf),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!("WSEN-TIDS: read I2C error: {}", e);
                Timer::after_secs(1).await;
                continue;
            }
            Err(_) => {
                warn!("WSEN-TIDS: read I2C timeout");
                Timer::after_secs(1).await;
                continue;
            }
        }
        let temp_raw = i16::from_le_bytes(buf);

        // temp_raw is in centidegrees (0.01 °C per LSB): 2500 = 25.00 °C
        info!(
            "temperature: {}.{:02} C",
            temp_raw / 100,
            (temp_raw % 100).abs()
        );

        let frame =
            Frame::new_data(StandardId::new(0x100).unwrap(), &temp_raw.to_be_bytes()).unwrap();

        // {
        //     let mut tx = can_tx.lock().await;
        //     match tx.try_write(&frame) {
        //         Ok(_) => {}
        //         Err(e) => trace!("CAN tx error: {}", e),
        //     }
        // }

        Timer::after_secs(1).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    info!("Hello World!");

    let status_led = Output::new(p.PC1, Level::High, Speed::Low);
    spawner.spawn(unwrap!(heartbeat_task(status_led)));

    let can_standby = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(can_standby); // keep the pin alive so it doesn't get deinitialized and go into standby mode

    let mut can = Can::new(p.CAN1, p.PB8, p.PB9, Irqs);
    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config().set_bitrate(1_000_000);
    can.enable().await;

    let (tx, rx) = can.split();
    let can_tx = CAN_TX.init(Mutex::new(tx));

    core::mem::forget(can); // keep the CAN peripheral alive so it doesn't get deinitialized

    spawner.spawn(unwrap!(can_rx_task(rx)));

    let i2c = i2c::I2c::new(
        p.I2C2,
        p.PB10, // SCL
        p.PB11, // SDA
        Irqs,
        p.DMA1_CH4,
        p.DMA1_CH5,
        Default::default(),
    );

    spawner.spawn(unwrap!(temperature_task(i2c, can_tx)));
}
