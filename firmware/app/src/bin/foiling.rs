#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::can::{
    Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler, TxInterruptHandler,
};
use embassy_stm32::peripherals::{self, CAN1};
use embassy_stm32::{bind_interrupts, dma};
use eoi_firmware::app_type::AppType;
use eoi_firmware::clock::clock_config;
use eoi_firmware::declare_app_type;
use eoi_firmware::display::{PanelMounting, run_display};
use {defmt_rtt as _, panic_probe as _};

declare_app_type!(AppType::FoilTuning);

// Has to stay in the binary: see the note in the dashboard binary.
bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    // Starboard side of the boat, in a mirrored case: the panel hangs upside
    // down, so the image is rotated 180 degrees to match.
    run_display(
        spawner,
        p,
        Irqs,
        MY_APP_TYPE,
        // The tuning table only while the helm is at the keyboard; see
        // `ScreenSelector`.
        draw_display::ScreenSelector::switching(
            draw_display::Layout::Foiling,
            draw_display::Layout::Information,
        ),
        PanelMounting::Inverted,
    )
    .await
}
