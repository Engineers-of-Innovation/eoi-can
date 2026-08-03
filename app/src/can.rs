use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::can::{
    BufferedCan, BufferedCanReceiver, Can, Fifo, RxBuf, TxBuf, filter::Mask32,
};
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use eoi_can_decoder::can_frame::CanFrame as DecoderFrame;
use eoi_can_decoder::{
    EoiBattery, EoiCanData, RudderControllerData, ServoData, parse_eoi_can_data,
};
use static_cell::StaticCell;

use crate::cooling_pump::BMS_DISCHARGE_STATE;
use crate::servo_rudder::{SERVO_COMMAND, SERVO_SETPOINT, SETPOINT_MAX, SETPOINT_MIN};
use crate::steering_angle::{CAN_ID_STEERING_CAL_CMD, CalCommand, STEERING_CAL_COMMAND};

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

                // Steering angle calibration command
                if let embassy_stm32::can::Id::Standard(id) = frame.id()
                    && id.as_raw() == CAN_ID_STEERING_CAL_CMD.as_raw()
                {
                    match frame.data().first().copied().and_then(CalCommand::from_u8) {
                        Some(command) => {
                            info!("Steering calibration command: {:?}", command);
                            STEERING_CAL_COMMAND.signal(command);
                        }
                        None => warn!("Unknown steering calibration command: {:02x}", frame.data()),
                    }
                }

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
