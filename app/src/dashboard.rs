//! Shared pieces for the dashboard binary: the e-paper draw target, the CAN
//! collector plumbing, and the bxCAN interrupt workarounds.
//!
//! Ported from `eoi-can-display-firmware` in the eoi-can repo.

use defmt::*;
use embassy_stm32::can::BufferedCanReceiver;
use embassy_stm32::gpio::Output;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use eoi_can_decoder::can_collector::CanCollector;
use eoi_can_decoder::can_frame::CanFrame;
use epd_waveshare::epd5in79::Display5in79;
use epd_waveshare::prelude::Color;

use crate::app_type::AppType;
use crate::can::handle_bootloader_reboot;

/// Frames the CAN RX task has drained but the render loop has not consumed yet.
///
/// `ThreadModeRawMutex` is sound here because both halves run on the same
/// thread-mode executor; nothing touches this from an interrupt.
pub static COLLECTOR: Mutex<ThreadModeRawMutex, CanCollector> = Mutex::new(CanCollector::new());

/// Wraps [`Display5in79`] so the shared `draw_display` code (which uses the
/// standard embedded-graphics convention, `On` = white) renders correctly on
/// the panel.
///
/// epd-waveshare's built-in `From<BinaryColor> for Color` maps `On -> Black`,
/// the opposite of the simulator/framebuffer. The epd5in79 driver writes the
/// buffer raw with `Color::White` = bit 1 = white on the panel, so without
/// this wrapper `clear(BinaryColor::On)` would paint a black background. It
/// maps `On -> White` / `Off -> Black` so all three outputs (firmware,
/// simulator, framebuffer) match. If the panel turns out inverted on the
/// bench, flip the mapping here — single point of change.
pub struct EpdDisplay(pub Display5in79);

impl EpdDisplay {
    pub fn new() -> Self {
        Self(Display5in79::default())
    }

    pub fn buffer(&self) -> &[u8] {
        self.0.buffer()
    }
}

impl Default for EpdDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginDimensions for EpdDisplay {
    fn size(&self) -> Size {
        self.0.size()
    }
}

impl DrawTarget for EpdDisplay {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        self.0
            .draw_iter(pixels.into_iter().map(|Pixel(point, color)| {
                let color = match color {
                    BinaryColor::On => Color::White,
                    BinaryColor::Off => Color::Black,
                };
                Pixel(point, color)
            }))
    }
}

/// FNV-1a over the framebuffer, so an unchanged frame can skip a panel drive cycle.
pub fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Defensive against two embassy-stm32 quirks that leave interrupts off
/// permanently (still present as of `embassy-stm32-v0.6.0`):
/// - the non-buffered RX ISR path clears `IER.FMPIE`, and the buffered path has
///   no counterpart that sets it again;
/// - the SCE handler clears `IER.ERRIE` on the first bus error and nothing ever
///   restores it.
///
/// Safe to call repeatedly. `FMPIE` is level-driven on `RFR.FMP != 0`, so if the
/// hardware FIFO already holds frames the RX ISR fires as soon as this returns
/// and drains them — no manual drain needed.
pub fn rearm_can_rx_interrupts() {
    embassy_stm32::pac::CAN1.ier().modify(|w| {
        w.set_fmpie(0, true);
        w.set_fmpie(1, true);
        w.set_errie(true);
    });
}

/// Distinguishes "quiet bus" from "our RX interrupt is off" from "we are
/// error-passive / bus-off".
pub fn log_can_state() {
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

/// Drains the CAN RX buffer into [`COLLECTOR`], blinking `activity_led` per frame.
///
/// The lock is held for exactly one insert so the render loop, which takes it
/// once per iteration, never blocks this task for long.
#[embassy_executor::task]
pub async fn dashboard_can_rx_task(
    rx: BufferedCanReceiver,
    mut activity_led: Output<'static>,
    app_type: AppType,
) {
    loop {
        match rx.receive().await {
            Ok(envelope) => {
                let frame = &envelope.frame;
                handle_bootloader_reboot(frame, app_type);

                // `from_encoded` takes a slice, which keeps the decoder's
                // heapless 0.8 `Vec` out of this crate (the app is on 0.9).
                let decoded = CanFrame::from_encoded(*frame.header().id(), frame.data());
                trace!("CAN frame: {}", decoded);
                COLLECTOR.lock().await.insert(decoded);
                activity_led.toggle();
            }
            Err(e) => warn!("CAN rx error: {:?}", e),
        }
    }
}
