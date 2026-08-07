use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::can::{
    BufferedCan, BufferedCanReceiver, Can, Fifo, RxBuf, TxBuf, filter::Mask32,
};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use eoi_can_decoder::can_frame::CanFrame as DecoderFrame;
use eoi_can_decoder::{EoiBattery, EoiCanData, parse_eoi_can_data};
use static_cell::StaticCell;

use crate::cooling_pump::BMS_DISCHARGE_STATE;

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

/// Reset into the bootloader if this frame is the host's REBOOT command *for this
/// board*.
///
/// Every application must call this on each received frame — it is the only way
/// the flash tool can get a running app back into the bootloader, so without it
/// OTA updates are one-way.
///
/// The command ID is derived from `app_type`, so rebooting one board leaves the
/// others running. Applications still use an accept-all hardware filter (the
/// dashboard needs the whole bus), so this check is what scopes the reset.
pub fn handle_bootloader_reboot(frame: &embassy_stm32::can::Frame, app_type: AppType) {
    if let embassy_stm32::can::Id::Standard(id) = frame.id()
        && id.as_raw() == protocol::board_address(app_type).cmd
        && frame.data().first() == Some(&protocol::msg::REBOOT)
    {
        info!("Reboot to bootloader requested via CAN");
        cortex_m::peripheral::SCB::sys_reset();
    }
}

#[embassy_executor::task]
pub async fn can_rx_task(rx: BufferedCanReceiver, app_type: AppType) {
    loop {
        match rx.receive().await {
            Ok(envelope) => {
                let frame = &envelope.frame;
                handle_bootloader_reboot(frame, app_type);

                let ec_id = match frame.id() {
                    embassy_stm32::can::Id::Standard(s) => embedded_can::Id::Standard(
                        embedded_can::StandardId::new(s.as_raw()).unwrap(),
                    ),
                    embassy_stm32::can::Id::Extended(e) => embedded_can::Id::Extended(
                        embedded_can::ExtendedId::new(e.as_raw()).unwrap(),
                    ),
                };
                let decoder_frame = DecoderFrame::from_encoded(ec_id, frame.data());
                if let Some(EoiCanData::EoiBattery(EoiBattery::TemperaturesAndStates(t))) =
                    parse_eoi_can_data(&decoder_frame)
                {
                    BMS_DISCHARGE_STATE.signal(t.discharge_state);
                }

                trace!("CAN rx: {:02x}", frame.data());
            }
            Err(e) => warn!("CAN rx error: {:?}", e),
        }
    }
}
