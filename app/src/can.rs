use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::can::{
    BufferedCan, BufferedCanReceiver, BufferedCanSender, Can, Fifo, RxBuf, TxBuf, filter::Mask32,
};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use eoi_can_decoder::can_frame::CanFrame as DecoderFrame;
use eoi_can_decoder::{
    EoiBattery, EoiCanData, RudderControllerData, ServoData, parse_eoi_can_data,
};
use static_cell::StaticCell;

use crate::cooling_pump::BMS_DISCHARGE_STATE;
use crate::servo_rudder::{SERVO_COMMAND, SERVO_SETPOINT, SETPOINT_MAX, SETPOINT_MIN};

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

#[embassy_executor::task]
pub async fn can_rx_task(rx: BufferedCanReceiver, mut tx: BufferedCanSender, app_type: AppType) {
    loop {
        match rx.receive().await {
            Ok(envelope) => {
                let frame = &envelope.frame;
                handle_bootloader_command(frame, app_type, &mut tx);

                let ec_id = match frame.id() {
                    embassy_stm32::can::Id::Standard(s) => embedded_can::Id::Standard(
                        embedded_can::StandardId::new(s.as_raw()).unwrap(),
                    ),
                    embassy_stm32::can::Id::Extended(e) => embedded_can::Id::Extended(
                        embedded_can::ExtendedId::new(e.as_raw()).unwrap(),
                    ),
                };
                let decoder_frame = DecoderFrame::from_encoded(ec_id, frame.data());
                match parse_eoi_can_data(&decoder_frame) {
                    Some(EoiCanData::EoiBattery(EoiBattery::TemperaturesAndStates(t))) => {
                        BMS_DISCHARGE_STATE.signal(t.discharge_state);
                    }
                    Some(EoiCanData::RudderController(RudderControllerData::Servo(
                        ServoData::Setpoint(setpoint),
                    ))) => {
                        if (SETPOINT_MIN..=SETPOINT_MAX).contains(&setpoint) {
                            SERVO_SETPOINT.signal(setpoint);
                        } else {
                            warn!("Servo setpoint {} out of range, rejected", setpoint);
                        }
                    }
                    Some(EoiCanData::RudderController(RudderControllerData::Servo(
                        ServoData::Command(command),
                    ))) => {
                        SERVO_COMMAND.signal(command);
                    }
                    _ => {}
                }

                trace!("CAN rx: {:02x}", frame.data());
            }
            Err(e) => warn!("CAN rx error: {:?}", e),
        }
    }
}
