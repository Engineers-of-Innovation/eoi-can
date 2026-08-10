use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::can::{
    BufferedCan, BufferedCanReceiver, BufferedCanSender, Can, Fifo, RxBuf, TxBuf, filter::Mask32,
};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use eoi_can_decoder::can_frame::CanFrame as DecoderFrame;
use eoi_can_decoder::{EoiCanData, parse_eoi_can_data};
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

use eoi_boot_api::header::AppType;
use eoi_boot_api::protocol;

use crate::build_info::VERSION;

/// Answer the host's bootloader-protocol commands aimed *at this board*, and
/// reset into the bootloader when asked.
///
/// Every application must call this on each received frame — the reboot case is
/// the only way the flash tool can get a running app back into the bootloader,
/// so without it OTA updates are one-way.
///
/// This is the driver half only; the decision lives in
/// [`protocol::app_action`], which is `no_std` and host-tested.
pub fn handle_bootloader_command(
    frame: &embassy_stm32::can::Frame,
    app_type: AppType,
    tx: &mut BufferedCanSender,
) {
    let embassy_stm32::can::Id::Standard(id) = frame.id() else {
        return;
    };
    match protocol::app_action(id.as_raw(), frame.data(), app_type, &VERSION) {
        Some(protocol::AppAction::Reply { id, data, len }) => reply(tx, id, &data[..len]),
        Some(protocol::AppAction::Reboot) => {
            info!("Reboot to bootloader requested via CAN");
            cortex_m::peripheral::SCB::sys_reset();
        }
        None => {}
    }
}

/// Queue a response frame without blocking.
///
/// `try_write`, not the async `write`: this runs inline in the RX loop, and on
/// the dashboard that loop must never stall behind a TX mailbox. A full TX
/// buffer means the host will retry anyway.
fn reply(tx: &mut BufferedCanSender, resp_id: u16, data: &[u8]) {
    let Some(id) = embassy_stm32::can::StandardId::new(resp_id) else {
        return;
    };
    let Ok(frame) = embassy_stm32::can::Frame::new_data(id, data) else {
        return;
    };
    if tx.try_write(frame).is_err() {
        warn!("CAN TX buffer full, dropped bootloader response");
    }
}

/// Decode a received frame into an application message, if it is one we know.
///
/// `from_encoded` takes a slice, which keeps the decoder's heapless 0.8 `Vec`
/// out of this crate (the app is on 0.9).
pub fn decode(frame: &embassy_stm32::can::Frame) -> Option<EoiCanData> {
    let ec_id = match frame.id() {
        embassy_stm32::can::Id::Standard(s) => {
            embedded_can::Id::Standard(embedded_can::StandardId::new(s.as_raw()).unwrap())
        }
        embassy_stm32::can::Id::Extended(e) => {
            embedded_can::Id::Extended(embedded_can::ExtendedId::new(e.as_raw()).unwrap())
        }
    };
    parse_eoi_can_data(&DecoderFrame::from_encoded(ec_id, frame.data()))
}

/// The minimal RX loop: answer bootloader commands, and nothing else.
///
/// This is all a board needs when it consumes no inbound application message —
/// the height-sensor-controller only ever transmits. Boards that do consume one
/// supply their own task so the handling lives with its consumers; see
/// [`crate::rudder_can::rudder_can_rx_task`] and
/// [`crate::dashboard::dashboard_can_rx_task`].
#[embassy_executor::task]
pub async fn can_rx_task(rx: BufferedCanReceiver, mut tx: BufferedCanSender, app_type: AppType) {
    loop {
        match rx.receive().await {
            Ok(envelope) => {
                let frame = &envelope.frame;
                handle_bootloader_command(frame, app_type, &mut tx);
                trace!("CAN rx: {:02x}", frame.data());
            }
            Err(e) => warn!("CAN rx error: {:?}", e),
        }
    }
}
