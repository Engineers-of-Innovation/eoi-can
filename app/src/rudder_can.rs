//! The rudder controller's inbound CAN handling.
//!
//! Split out of the shared [`crate::can::can_rx_task`] because these messages
//! only mean something on this board: the accept-all hardware filter means every
//! board sees them, so a board-agnostic handler had the height-sensor-controller
//! decoding setpoints and warning about rudder traffic it has no part in.

use defmt::*;
use embassy_stm32::can::{BufferedCanReceiver, BufferedCanSender};
use eoi_boot_api::header::AppType;
use eoi_can_decoder::{EoiBattery, EoiCanData, RudderControllerData, ServoData};

use crate::can::{decode, handle_bootloader_command};
use crate::cooling_pump::BMS_DISCHARGE_STATE;
use crate::servo_rudder::{SERVO_COMMAND, SERVO_SETPOINT, SETPOINT_MAX, SETPOINT_MIN};

#[embassy_executor::task]
pub async fn rudder_can_rx_task(
    rx: BufferedCanReceiver,
    mut tx: BufferedCanSender,
    app_type: AppType,
) {
    loop {
        match rx.receive().await {
            Ok(envelope) => {
                let frame = &envelope.frame;
                handle_bootloader_command(frame, app_type, &mut tx);

                match decode(frame) {
                    Some(EoiCanData::EoiBattery(EoiBattery::TemperaturesAndStates(t))) => {
                        BMS_DISCHARGE_STATE.signal(t.discharge_state);
                    }
                    Some(EoiCanData::RudderController(RudderControllerData::Servo(
                        ServoData::Setpoint(setpoint),
                    ))) => {
                        // The range gate belongs here rather than in
                        // `servo_rudder`: an out-of-range setpoint must not feed
                        // the 2 s watchdog (docs/rudder-servo.md), whereas
                        // `setpoint_to_steps` clamps instead of rejecting.
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
