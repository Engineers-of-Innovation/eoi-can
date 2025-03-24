#![no_std]
#![no_main]

use defmt::{info, warn};
use draw_display::can_frame::CanFrame;
use embassy_executor::Spawner;
use embassy_stm32::can::filter::Mask32;
use embassy_stm32::can::{
    Can, Fifo, Frame, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler,
    TxInterruptHandler,
};
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::peripherals::CAN1;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, spi, Peripherals};
use embassy_time::{Delay, Timer};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct CanInterrupts {
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    CAN1_TX => TxInterruptHandler<CAN1>;
});

use epd_waveshare::{
    // epd2in9_v2::{Display2in9, Epd2in9},  // board now used to connect 5in7 display
    epd7in5_v2::{Display7in5, Epd7in5},
    prelude::*,
};

pub fn embassy_init() -> Peripherals {
    use embassy_stm32::rcc::{Pll, PllMul, PllPreDiv, PllRDiv, PllSource};

    let mut config = embassy_stm32::Config::default();
    let mut mux = embassy_stm32::rcc::mux::ClockMux::default();
    mux.adcsel = embassy_stm32::rcc::mux::Adcsel::SYS;
    config.rcc = embassy_stm32::rcc::Config {
        msi: None,
        hsi: false,
        hse: Some(embassy_stm32::rcc::Hse {
            freq: Hertz::mhz(16),
            mode: embassy_stm32::rcc::HseMode::Oscillator,
        }),
        sys: embassy_stm32::rcc::Sysclk::PLL1_R,
        // run everything on 64 Mhz
        pll: Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL8,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV2),
        }),
        pllsai1: Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL8,
            divp: None,
            divq: Some(embassy_stm32::rcc::PllQDiv::DIV2),
            divr: None,
        }),
        pllsai2: Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL8,
            divp: None,
            divq: Some(embassy_stm32::rcc::PllQDiv::DIV2),
            divr: None,
        }),
        mux,
        ahb_pre: embassy_stm32::rcc::AHBPrescaler::DIV1,
        apb1_pre: embassy_stm32::rcc::APBPrescaler::DIV1,
        apb2_pre: embassy_stm32::rcc::APBPrescaler::DIV1,
        ls: embassy_stm32::rcc::LsConfig {
            rtc: embassy_stm32::rcc::RtcClockSource::LSE,
            lsi: false,
            lse: Some(embassy_stm32::rcc::LseConfig {
                frequency: Hertz::hz(32768),
                mode: embassy_stm32::rcc::LseMode::Oscillator(
                    embassy_stm32::rcc::LseDrive::MediumHigh,
                ),
            }),
        },
    };

    embassy_stm32::init(config)
}

#[embassy_executor::task]
pub async fn can_receiver(mut can_rx: embassy_stm32::can::CanRx<'static>) {
    info!("waiting for can data");
    loop {
        let envelope = can_rx.read().await;
        if let Ok(envelope) = envelope {
            let data_len = envelope.frame.header().len() as usize;
            let data_slice = &envelope.frame.data()[..data_len];
            let data_vec: heapless::Vec<u8, 8> = heapless::Vec::from_slice(data_slice)
                .expect("CAN messages are at most 8 bytes, so this should never fail");
            let frame = CanFrame {
                id: *envelope.frame.header().id(),
                data: data_vec,
            };
            info!("CAN frame: {}", frame);
        } else if let Err(try_read) = envelope {
            info!("CAN frame try read error: {}", try_read);
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_init();
    info!("Hello Rust!");

    // leds are low active
    let mut led_green = Output::new(p.PC1, Level::High, Speed::Low);
    let mut led_red = Output::new(p.PC2, Level::High, Speed::Low);
    let mut led_blue = Output::new(p.PC3, Level::High, Speed::Low);

    led_red.set_low();

    let busy = Input::new(p.PA8, Pull::Down);

    let dc = Output::new(p.PC9, Level::High, Speed::VeryHigh);

    let reset = Output::new(p.PC8, Level::Low, Speed::VeryHigh);

    let spi = spi::Spi::new_blocking(p.SPI2, p.PB13, p.PB15, p.PB14, spi::Config::default());

    let cs = Output::new(p.PB6, Level::High, Speed::VeryHigh);

    let mut spi_device = embedded_hal_bus::spi::ExclusiveDevice::new(spi, cs, Delay).unwrap();

    let can_standby = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(can_standby);
    let mut can = Can::new(p.CAN1, p.PB8, p.PB9, CanInterrupts);
    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config().set_loopback(false).set_silent(false);
    can.set_bitrate(500_000);
    can.set_tx_fifo_scheduling(true);
    can.enable().await;
    let (mut can_tx, can_rx) = can.split();

    spawner.must_spawn(can_receiver(can_rx));

    // Delay.delay_ms(1000);

    // info!("Init display");

    // let mut epd = Epd7in5::new(&mut spi_device, busy, dc, reset, &mut Delay, Some(1000)).unwrap();

    // info!("Init done");

    // epd.clear_frame(&mut spi_device, &mut Delay).unwrap();

    // // currently display colors are inverted, not sure what is going on, changing BWRBIT does not do much
    // let mut display = Display7in5::default();

    led_red.set_high();
    led_green.set_low();

    // draw_display::draw_display(&mut display).unwrap();

    // epd.update_and_display_frame(&mut spi_device, display.buffer(), &mut Delay)
    //     .unwrap();

    loop {
        led_blue.toggle();

        let result = can_tx.try_write(&Frame::new_standard(0x123, &[0, 1, 2, 3]).unwrap());
        if result.is_ok() {
            info!("send can message");
        } else if let Err(error) = result {
            warn!("fail to send can with error {}", error);
        }

        Timer::after_secs(1).await;
    }
}
