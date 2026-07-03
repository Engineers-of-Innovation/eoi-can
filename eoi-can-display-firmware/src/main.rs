#![no_std]
#![no_main]

#[allow(unused_imports)]
use defmt::{debug, error, info, trace, warn};
use embassy_executor::Spawner;
use embassy_stm32::can::enums::BusError;
use embassy_stm32::can::filter::Mask32;
use embassy_stm32::can::{
    BufferedCanRx, Can, Fifo, Rx0InterruptHandler, Rx1InterruptHandler, RxBuf, SceInterruptHandler,
    TxInterruptHandler,
};
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::peripherals::CAN1;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, spi, Peripherals};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Delay, Timer};
use eoi_can_decoder::can_collector::CanCollector;
use eoi_can_decoder::can_frame::CanFrame;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct CanInterrupts {
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    CAN1_TX => TxInterruptHandler<CAN1>;
});

use epd_waveshare::epd5in79::Epd5in79;

mod inverted_display;
use inverted_display::EpdDisplay;

/// Software RX buffer size for the buffered CAN receiver. The buffered driver's
/// ISR drains the 3-deep bxCAN hardware FIFO into this channel independently of
/// the executor. `can_receiver` then drains the channel into the collector.
///
/// The display refresh's long BUSY-pin wait is async (it yields), so
/// `can_receiver` keeps draining throughout it. The only windows where the
/// channel must buffer on its own are the blocking SPI framebuffer transfers
/// (~200-400 ms total per refresh). This size gives ample headroom for that;
/// bump it if `Dropped frames` climbs under sustained heavy bus load.
const CAN_RX_BUF_SIZE: usize = 256;

static SHARED_CAN_COLLECTOR: Mutex<ThreadModeRawMutex, CanCollector> =
    Mutex::new(CanCollector::new());

static RX_BUF: StaticCell<RxBuf<CAN_RX_BUF_SIZE>> = StaticCell::new();

const ENABLE_SKIP_DISPLAY_UPDATE_IF_UNCHANGED: bool = true;

/// FNV-1a 64-bit hash, used to detect when the rendered framebuffer is
/// unchanged so we can skip an unnecessary panel refresh.
fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

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
pub async fn can_receiver(
    mut can_rx: BufferedCanRx<'static, CAN_RX_BUF_SIZE>,
    mut output_led: Output<'static>,
) {
    let mut last_bus_error: Option<BusError> = None;
    loop {
        let envelope = can_rx.read().await;
        if let Ok(envelope) = envelope {
            last_bus_error = None;
            let data_len = envelope.frame.header().len() as usize;
            let data_slice = &envelope.frame.data()[..data_len];
            let data_vec: heapless::Vec<u8, 8> = heapless::Vec::from_slice(data_slice)
                .expect("CAN messages are at most 8 bytes, so this should never fail");
            let frame = CanFrame {
                id: *envelope.frame.header().id(),
                data: data_vec,
            };
            trace!("CAN frame: {}", frame);
            SHARED_CAN_COLLECTOR.lock().await.insert(frame);
            output_led.toggle();
        } else if let Err(bus_error) = envelope {
            // Compare the discriminant to avoid needing PartialEq
            let is_same_error = match last_bus_error {
                Some(ref last) => {
                    core::mem::discriminant(last) == core::mem::discriminant(&bus_error)
                }
                None => false,
            };
            if is_same_error {
                error!("CAN frame try read error: {}", bus_error);
            }
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
    let led_blue = Output::new(p.PC3, Level::High, Speed::Low);

    led_red.set_low();

    let busy = Input::new(p.PA8, Pull::Down);
    let dc = Output::new(p.PC9, Level::High, Speed::VeryHigh);
    let reset = Output::new(p.PC8, Level::Low, Speed::VeryHigh);

    let mut spi_config = spi::Config::default();
    spi_config.frequency = Hertz::mhz(2); // max 5 on display
    // Async DMA SPI so the large framebuffer transfers yield to the executor
    // (letting `can_receiver` run) instead of blocking it. SPI2 TX = DMA1_CH5,
    // RX = DMA1_CH4 (both free; CAN1 uses no DMA).
    let spi = spi::Spi::new(
        p.SPI2,
        p.PB13,
        p.PB15,
        p.PB14,
        p.DMA1_CH5,
        p.DMA1_CH4,
        spi_config,
    );

    let cs = Output::new(p.PC6, Level::High, Speed::VeryHigh);

    let mut spi_device = embedded_hal_bus::spi::ExclusiveDevice::new(spi, cs, Delay).unwrap();

    let can_standby = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(can_standby);
    let mut can = Can::new(p.CAN1, p.PB8, p.PB9, CanInterrupts);
    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config().set_loopback(false).set_silent(false);
    can.set_bitrate(1_000_000);
    can.set_tx_fifo_scheduling(true);
    can.enable().await;
    let (_, can_rx) = can.split();
    let buffered_rx = can_rx.buffered(RX_BUF.init(RxBuf::new()));

    spawner.must_spawn(can_receiver(buffered_rx, led_blue));

    Timer::after_secs(1).await;

    info!("Init display");

    let mut epd = Epd5in79::new_async(&mut spi_device, busy, dc, reset, &mut Delay, Some(1000))
        .await
        .unwrap();

    info!("Init done");

    led_red.set_high();

    let mut display = EpdDisplay::new();
    let mut display_data = draw_display::DisplayData::default();
    draw_display::draw_display(&mut display, &display_data).unwrap();

    epd.update_and_display_frame_async(&mut spi_device, display.buffer(), &mut Delay)
        .await
        .unwrap();

    // The loop refreshes quickly (differential partial refresh, ~0.5 s) and
    // periodically does a clean full refresh to clear the ghosting that fast
    // mode accumulates. Start the counter at the threshold so the first loop
    // iteration (the first paint of real CAN data) is a full refresh, then
    // switches to fast mode.
    //
    // FULL_REFRESH_EVERY: number of quick refreshes between clean full
    // refreshes (~every 1 min at the 1 s cadence below). Tune if ghosting
    // appears too early or the full-refresh flash is too frequent.
    const FULL_REFRESH_EVERY: u32 = 60;
    let mut quick_count: u32 = FULL_REFRESH_EVERY;
    // Hash of the last frame actually pushed to the panel. Refreshing
    // identical content is pointless (and every refresh stresses the panel),
    // so we skip the refresh entirely when the rendered frame hasn't changed.
    let mut last_buffer_hash: Option<u64> = None;

    info!("Starting main loop");

    loop {
        info!("Decoding CAN data");
        // Take the collected frames out under the lock and release it
        // immediately, so the high-priority `can_receiver` task is blocked on the
        // shared collector for as short as possible. `mem::take` leaves an empty
        // collector behind for the receiver to keep filling while we decode.
        let snapshot = {
            let mut can_collector = SHARED_CAN_COLLECTOR.lock().await;
            if can_collector.get_dropped_frames() > 0 {
                debug!("Dropped frames: {}", can_collector.get_dropped_frames());
            }
            core::mem::take(&mut *can_collector)
        };

        let mut parsed_frames = 0_u32;
        snapshot.iter().for_each(|frame| {
            trace!("Paring CAN frame: {:?}", frame);
            if let Some(parsed_data) = eoi_can_decoder::parse_eoi_can_data(frame) {
                display_data.ingest_eoi_can_data(parsed_data);
                parsed_frames = parsed_frames.saturating_add(1);
            } else {
                warn!("Failed to parse data from CAN frame: {:?}", frame);
            }
        });
        debug!("Parsed frames: {}", parsed_frames);

        draw_display::draw_display(&mut display, &display_data).unwrap();
        let buffer_hash = fnv1a_hash(display.buffer());

        if last_buffer_hash == Some(buffer_hash) && ENABLE_SKIP_DISPLAY_UPDATE_IF_UNCHANGED {
            debug!("Display unchanged, skipping refresh");
        } else {
            // led_green.set_low();
            if quick_count >= FULL_REFRESH_EVERY {
                info!("Updating display (full refresh)");
                // Clean full refresh: re-init the panel (wake_up runs the
                // standard init), then paint. This also rewrites the "old"
                // RAM with the frame, re-establishing the baseline the
                // following partial refreshes diff against.
                epd.wake_up_async(&mut spi_device, &mut Delay)
                    .await
                    .unwrap();
                epd.update_and_display_frame_async(&mut spi_device, display.buffer(), &mut Delay)
                    .await
                    .unwrap();
                quick_count = 0;
            } else {
                info!("Updating display (quick refresh)");
                // Differential partial refresh: only the pixels that changed
                // versus the previous frame are driven.
                epd.display_partial_async(&mut spi_device, display.buffer(), &mut Delay)
                    .await
                    .unwrap();
                quick_count += 1;
            }
            last_buffer_hash = Some(buffer_hash);
            info!("Display updated");
            // led_green.set_high();
        }

        Timer::after_secs(1).await;
    }
}
