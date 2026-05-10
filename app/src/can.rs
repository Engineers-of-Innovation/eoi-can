use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::can::{
    BufferedCan, BufferedCanReceiver, Can, Fifo, RxBuf, TxBuf, filter::Mask32,
};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use static_cell::StaticCell;

static CAN: StaticCell<Can<'static>> = StaticCell::new();
static TX_BUF: StaticCell<TxBuf<8>> = StaticCell::new();
static RX_BUF: StaticCell<RxBuf<8>> = StaticCell::new();

pub async fn init_can(
    can: Can<'static>,
    standby: Peri<'_, impl Pin>,
) -> BufferedCan<'static, 8, 8> {
    let standby_out = Output::new(standby, Level::Low, Speed::Low);
    core::mem::forget(standby_out);

    let can = CAN.init(can);
    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config().set_bitrate(1_000_000);
    can.enable().await;

    can.buffered(TX_BUF.init(TxBuf::new()), RX_BUF.init(RxBuf::new()))
}

use eoi_boot_api::protocol;

#[embassy_executor::task]
pub async fn can_rx_task(rx: BufferedCanReceiver) {
    loop {
        match rx.receive().await {
            Ok(envelope) => {
                let frame = &envelope.frame;
                // Check for bootloader reboot command
                if let embassy_stm32::can::Id::Standard(id) = frame.id()
                    && id.as_raw() == protocol::CAN_ID_HOST_TO_DEVICE
                        && frame.data().first() == Some(&protocol::msg::REBOOT)
                    {
                        info!("Reboot to bootloader requested via CAN");
                        cortex_m::peripheral::SCB::sys_reset();
                    }
                trace!("CAN rx: {:02x}", frame.data());
            }
            Err(e) => warn!("CAN rx error: {:?}", e),
        }
    }
}
