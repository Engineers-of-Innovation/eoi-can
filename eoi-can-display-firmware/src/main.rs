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
use embassy_time::{Delay, Duration, Ticker, Timer};
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

/// The refresh's BUSY wait is async, so `can_receiver` keeps draining through
/// it; this only has to cover the blocking SPI framebuffer transfers. Bump it if
/// `Dropped frames` climbs under sustained heavy bus load.
const CAN_RX_BUF_SIZE: usize = 256;

static SHARED_CAN_COLLECTOR: Mutex<ThreadModeRawMutex, CanCollector> =
    Mutex::new(CanCollector::new());

static RX_BUF: StaticCell<RxBuf<CAN_RX_BUF_SIZE>> = StaticCell::new();

const ENABLE_SKIP_DISPLAY_UPDATE_IF_UNCHANGED: bool = true;

fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Defensive against two embassy-stm32 0.2.0 quirks that leave interrupts off
/// permanently:
/// - the non-buffered RX ISR path clears `IER.FMPIE`, and the buffered path has
///   no counterpart that sets it again;
/// - the SCE handler clears `IER.ERRIE` on the first bus error, and only
///   `try_read` (which panics in buffered mode) re-enables it.
///
/// Safe to call repeatedly. `FMPIE` is level-driven on `RFR.FMP != 0`, so if the
/// hardware FIFO already holds frames the RX ISR fires as soon as this returns
/// and drains them — no manual drain needed.
fn rearm_can_rx_interrupts() {
    embassy_stm32::pac::CAN1.ier().modify(|w| {
        w.set_fmpie(0, true);
        w.set_fmpie(1, true);
        w.set_errie(true);
    });
}

/// Distinguishes "quiet bus" from "our RX interrupt is off" from "we are
/// error-passive / bus-off".
fn log_can_state() {
    let can = embassy_stm32::pac::CAN1;
    let ier = can.ier().read();
    let rfr = can.rfr(0).read();
    let esr = can.esr().read();
    warn!(
        "CAN state: fmpie0={} errie={} fifo0(fmp={} full={} fovr={}) esr(boff={} epvf={} ewgf={} lec={} tec={} rec={})",
        ier.fmpie(0),
        ier.errie(),
        rfr.fmp(),
        rfr.full(),
        rfr.fovr(),
        esr.boff(),
        esr.epvf(),
        esr.ewgf(),
        esr.lec().to_bits(),
        esr.tec(),
        esr.rec(),
    );
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
            // Unreachable in buffered mode: `BufferedCanRx::read` just awaits the
            // channel and the RX ISR only ever sends `Ok`. Bus errors are
            // surfaced by the RX-stall health check in `main` instead. Kept so
            // the arm stays correct if this ever moves back to `CanRx`.
            //
            // Compare the discriminant to avoid needing PartialEq
            let is_same_error = match last_bus_error {
                Some(ref last) => {
                    core::mem::discriminant(last) == core::mem::discriminant(&bus_error)
                }
                None => false,
            };
            if !is_same_error {
                error!("CAN frame try read error: {}", bus_error);
            }
            last_bus_error = Some(bus_error);
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

    let epd_busy = Input::new(p.PA8, Pull::Down);
    let epd_pwr = Output::new(p.PB12, Level::High, Speed::High);
    core::mem::forget(epd_pwr); // keep power on for display
    let epd_dc = Output::new(p.PC9, Level::High, Speed::VeryHigh);
    let epd_reset = Output::new(p.PC8, Level::Low, Speed::VeryHigh);

    let mut spi_config = spi::Config::default();
    // Above the <=2 MHz every Waveshare reference for this panel uses; verified
    // on this hardware. Drop back to 2 if output ever tears or corrupts.
    spi_config.frequency = Hertz::mhz(4);
    // DMA so the framebuffer transfers yield to the executor instead of blocking
    // it. SPI2 TX = DMA1_CH5, RX = DMA1_CH4, both free (CAN1 uses no DMA).
    let epd_spi = spi::Spi::new(
        p.SPI2, p.PB13, p.PB15, p.PB14, p.DMA1_CH5, p.DMA1_CH4, spi_config,
    );

    let epd_cs = Output::new(p.PC6, Level::High, Speed::VeryHigh);

    let mut epd_spi_device =
        embedded_hal_bus::spi::ExclusiveDevice::new(epd_spi, epd_cs, Delay).unwrap();

    let can_standby = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(can_standby); // hold the transceiver out of standby
    let mut can = Can::new(p.CAN1, p.PB8, p.PB9, CanInterrupts);

    // Must switch RxMode to Buffered *before* the bitrate is programmed: the
    // peripheral is already live by then, and a frame arriving while RxMode is
    // still NonBuffered takes an ISR path that clears IER.FMPIE0 — which nothing
    // sets again in Buffered mode. RX would be dead for the rest of the boot.
    let (_, can_rx) = can.split();
    let buffered_rx = can_rx.buffered(RX_BUF.init(RxBuf::new()));

    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config().set_loopback(false).set_silent(false);
    can.set_bitrate(1_000_000);
    can.set_tx_fifo_scheduling(true);
    can.enable().await;

    // Redundant given the ordering above, but free.
    rearm_can_rx_interrupts();

    spawner.must_spawn(can_receiver(buffered_rx, led_blue));

    Timer::after_secs(1).await;

    info!("Init display");

    let mut epd = Epd5in79::new_async(
        &mut epd_spi_device,
        epd_busy,
        epd_dc,
        epd_reset,
        &mut Delay,
        Some(1000),
    )
    .await
    .unwrap();

    info!("Init done");

    led_red.set_high();

    let mut display = EpdDisplay::new();
    let mut display_data = draw_display::DisplayData::default();
    draw_display::draw_display(&mut display, &display_data).unwrap();

    epd.update_and_display_frame_async(&mut epd_spi_device, display.buffer(), &mut Delay)
        .await
        .unwrap();

    // The quick refresh drives every pixel, but only a mode-1 full refresh clears
    // the ghosting that mode 2 accumulates. Starting at the threshold makes the
    // first paint of real CAN data a full refresh.
    const FULL_REFRESH_EVERY: u32 = 60;
    let mut quick_count: u32 = FULL_REFRESH_EVERY;
    // Skipping identical frames spares the panel a drive cycle.
    let mut last_buffer_hash: Option<u64> = None;
    // Detects a stalled RX path instead of rendering stale-value dashes forever.
    // Iterations, not seconds — the loop period is work-bound.
    const RX_STALL_ITERATIONS: u32 = 5;
    let mut empty_snapshots: u32 = 0;

    // A floor on the loop period, not a delay added to it: the refresh alone
    // exceeds this, so every tick is already overdue and the loop runs work-bound.
    // The floor matters only when the frame is unchanged and the refresh is
    // skipped, where it stops the loop spinning on the CAN collector lock.
    const MIN_LOOP_PERIOD: Duration = Duration::from_millis(500);
    let mut ticker = Ticker::every(MIN_LOOP_PERIOD);

    info!("Starting main loop");

    loop {
        info!("Decoding CAN data");
        // `mem::take` leaves an empty collector for `can_receiver` to keep filling
        // while we decode, so it is blocked on the lock for as short as possible.
        let snapshot = {
            let mut can_collector = SHARED_CAN_COLLECTOR.lock().await;
            if can_collector.get_dropped_frames() > 0 {
                debug!("Dropped frames: {}", can_collector.get_dropped_frames());
            }
            core::mem::take(&mut *can_collector)
        };

        // Counted on raw frames rather than `parsed_frames`, so a bus carrying only
        // IDs we don't decode still counts as healthy.
        if snapshot.iter().next().is_some() {
            empty_snapshots = 0;
        } else {
            empty_snapshots = empty_snapshots.saturating_add(1);
            // Monotone counter, so the reported count is the real stall length
            // rather than resetting on every retry.
            if empty_snapshots % RX_STALL_ITERATIONS == 0 {
                warn!(
                    "No CAN frames received for {} loop iterations, re-arming RX interrupts",
                    empty_snapshots
                );
                log_can_state();
                rearm_can_rx_interrupts();
            }
        }

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
            led_green.set_low();
            if quick_count >= FULL_REFRESH_EVERY {
                info!("Updating display (full refresh)");
                // wake_up re-runs the standard init, which the mode-1 waveform
                // needs after a run of differential refreshes.
                epd.wake_up_async(&mut epd_spi_device, &mut Delay)
                    .await
                    .unwrap();
                epd.update_and_display_frame_async(
                    &mut epd_spi_device,
                    display.buffer(),
                    &mut Delay,
                )
                .await
                .unwrap();
                quick_count = 0;
            } else {
                info!("Updating display (quick refresh)");
                epd.display_refresh_all_async(&mut epd_spi_device, display.buffer(), &mut Delay)
                    .await
                    .unwrap();
                quick_count += 1;
            }
            last_buffer_hash = Some(buffer_hash);
            info!("Display updated");
            led_green.set_high();
        }

        ticker.next().await;
    }
}
