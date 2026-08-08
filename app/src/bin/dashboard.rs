#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::yield_now;
use embassy_stm32::time::Hertz;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{
    bind_interrupts,
    can::{
        Can, Fifo, Rx0InterruptHandler, Rx1InterruptHandler, RxBuf, SceInterruptHandler, TxBuf,
        TxInterruptHandler, filter::Mask32,
    },
    dma,
    gpio::{Input, Level, Output, Pull, Speed},
    peripherals::{self, CAN1},
    spi,
};
use embassy_time::{Delay, Duration, Ticker, Timer};
use eoi_rust_firmware::app_type::AppType;
use eoi_rust_firmware::clock::clock_config;
use eoi_rust_firmware::dashboard::{
    COLLECTOR, EpdDisplay, dashboard_can_rx_task, fnv1a_hash, log_can_state,
    rearm_can_rx_interrupts,
};
use eoi_rust_firmware::declare_app_type;
use epd_waveshare::epd5in79::Epd5in79;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

declare_app_type!(AppType::Dashboard);

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
});

/// Deep enough to cover a full render pass. `draw_display` is synchronous, and a
/// saturated 1 Mbps bus delivers a frame roughly every 120 us, so a shallow
/// buffer would drop frames while the framebuffer is being redrawn. Bump it if
/// `Dropped frames` climbs under sustained heavy bus load.
const CAN_RX_BUF_SIZE: usize = 256;

static RX_BUF: StaticCell<RxBuf<CAN_RX_BUF_SIZE>> = StaticCell::new();
static TX_BUF: StaticCell<TxBuf<4>> = StaticCell::new();
/// The framebuffer is an inline `[u8; 26928]`. `init_with` builds it in place;
/// passing it by value risks a 27 KB stack temporary at `opt-level = "z"`.
static DISPLAY: StaticCell<EpdDisplay> = StaticCell::new();

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
    info!("Dashboard");
    eoi_rust_firmware::build_info::log();

    // LEDs are active low.
    let green_led = Output::new(p.PC1, Level::High, Speed::Low);
    let mut red_led = Output::new(p.PC2, Level::High, Speed::Low);
    let blue_led = Output::new(p.PC3, Level::High, Speed::Low);

    // Red stays on until the panel is initialised, then marks each refresh.
    red_led.set_low();

    let watchdog = IndependentWatchdog::new(p.IWDG, 4_000_000);
    spawner.spawn(unwrap!(heartbeat_task(green_led, watchdog)));

    let epd_busy = Input::new(p.PA8, Pull::Down);
    let epd_pwr = Output::new(p.PB12, Level::High, Speed::High);
    core::mem::forget(epd_pwr); // keep power on for display
    let epd_dc = Output::new(p.PC9, Level::High, Speed::VeryHigh);
    let epd_reset = Output::new(p.PC8, Level::Low, Speed::VeryHigh);

    let mut spi_config = spi::Config::default();
    // 2.5 MHz, not the 4 MHz the eoi-can firmware used. That firmware ran a
    // 64 MHz sysclk where 4 MHz fell out exactly (DIV16); this board shares the
    // fleet's 80 MHz config, and embassy rounds to a divisor bucket that can
    // overshoot — asking for 4 MHz here would silently give 5 MHz, above the
    // fastest rate bench-verified on this panel. 80/32 lands on 2.5 MHz exactly,
    // still above the <=2 MHz every Waveshare reference uses. The cost is ~86 ms
    // per full framebuffer transfer instead of ~54 ms, and it is DMA-backed, so
    // the CAN RX task keeps draining through it.
    spi_config.frequency = Hertz::khz(2_500);
    // DMA so the framebuffer transfers yield to the executor instead of blocking
    // it. SPI2 TX = DMA1_CH5, RX = DMA1_CH4 — the only pair this SPI can use,
    // which is why the dashboard has no I2C temperature sensor.
    let epd_spi = spi::Spi::new(
        p.SPI2, p.PB13, p.PB15, p.PB14, p.DMA1_CH5, p.DMA1_CH4, Irqs, spi_config,
    );

    let epd_cs = Output::new(p.PC6, Level::High, Speed::VeryHigh);
    let mut epd_spi_device =
        embedded_hal_bus::spi::ExclusiveDevice::new(epd_spi, epd_cs, Delay).unwrap();

    let can_standby = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(can_standby); // hold the transceiver out of standby
    let mut can = Can::new(p.CAN1, p.PB8, p.PB9, Irqs);

    // Must switch RxMode to Buffered *before* the bitrate is programmed: the
    // peripheral is already live by then, and a frame arriving while RxMode is
    // still NonBuffered takes an ISR path that clears IER.FMPIE0 — which nothing
    // sets again in Buffered mode. RX would be dead for the rest of the boot.
    let (can_tx, can_rx) = can.split();
    let buffered_rx = can_rx.buffered(RX_BUF.init(RxBuf::new()));
    // The dashboard originates no traffic; the TX half exists only so it can
    // answer the host's bootloader-protocol queries (state, version). Four
    // frames is ample — the host asks one question at a time.
    let buffered_tx = can_tx.buffered(TX_BUF.init(TxBuf::new()));

    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config()
        .set_loopback(false)
        .set_silent(false)
        .set_bitrate(1_000_000);
    can.enable().await;

    // Redundant given the ordering above, but free.
    rearm_can_rx_interrupts();

    spawner.spawn(unwrap!(dashboard_can_rx_task(
        buffered_rx.reader(),
        buffered_tx.writer(),
        blue_led,
        MY_APP_TYPE
    )));

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
    red_led.set_high();

    let display = DISPLAY.init_with(EpdDisplay::new);
    let mut display_data = draw_display::DisplayData::default();
    draw_display::draw_display(display, &display_data).unwrap();

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

    // A floor on the loop period. Must stay comfortably above the ~490 ms that
    // `draw_display` + `fnv1a_hash` cost, or the ticker never gets to wait:
    // `Ticker::next` advances `expires_at` by exactly one period per call, so
    // once it falls behind it only claws back (period - body) per iteration and
    // returns `Ready` immediately until it catches up. With a 500 ms period that
    // margin was 9 ms, and the 2.5 s full refresh put it ~5 periods in debt, so
    // it never did — see the `yield_now` below for why that was fatal.
    const MIN_LOOP_PERIOD: Duration = Duration::from_secs(1);
    let mut ticker = Ticker::every(MIN_LOOP_PERIOD);

    info!("Starting main loop");

    loop {
        // `mem::take` leaves an empty collector for the RX task to keep filling
        // while we decode, so it is blocked on the lock for as short as possible.
        let snapshot = {
            let mut collector = COLLECTOR.lock().await;
            if collector.get_dropped_frames() > 0 {
                debug!("Dropped frames: {}", collector.get_dropped_frames());
            }
            core::mem::take(&mut *collector)
        };

        // Counted on raw frames rather than parsed ones, so a bus carrying only
        // IDs we don't decode still counts as healthy.
        if snapshot.iter().next().is_some() {
            empty_snapshots = 0;
        } else {
            empty_snapshots = empty_snapshots.saturating_add(1);
            // Monotone counter, so the reported count is the real stall length
            // rather than resetting on every retry.
            if empty_snapshots.is_multiple_of(RX_STALL_ITERATIONS) {
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
            if let Some(parsed_data) = eoi_can_decoder::parse_eoi_can_data(frame) {
                display_data.ingest_eoi_can_data(parsed_data);
                parsed_frames = parsed_frames.saturating_add(1);
            } else {
                trace!("Failed to parse data from CAN frame: {:?}", frame);
            }
        });
        debug!("Parsed frames: {}", parsed_frames);

        draw_display::draw_display(display, &display_data).unwrap();
        let buffer_hash = fnv1a_hash(display.buffer());

        if last_buffer_hash == Some(buffer_hash) {
            debug!("Display unchanged, skipping refresh");
        } else {
            red_led.set_low();
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
            red_led.set_high();
            // A refresh takes 0.7-2.5 s, many periods' worth. Without this the
            // ticker carries that debt forward and stops yielding afterwards.
            ticker.reset();
        }

        // The one await in this loop guaranteed to return `Pending`, and the
        // reason the loop is sound at all. Everything else can complete inline:
        // `COLLECTOR.lock()` is uncontended, the refresh branch is skipped
        // whenever the frame is unchanged, and `ticker.next()` returns `Ready`
        // while it is behind. An embassy task that never returns `Pending` is
        // never descheduled, which previously starved `heartbeat_task` (so the
        // 4 s watchdog reset the board) and `dashboard_can_rx_task` (so the
        // collector stayed empty and the RX path looked dead from the inside).
        yield_now().await;

        ticker.next().await;
    }
}
