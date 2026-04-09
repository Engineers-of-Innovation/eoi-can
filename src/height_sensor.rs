use defmt::*;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::Input;
use embassy_stm32::mode::Async;
use embassy_stm32::usart;
use embassy_time::{Duration, Timer, with_timeout};
use rmodbus::{ModbusProto, client::ModbusRequest};

pub const CAN_ID_HEIGHT_SENSOR_FRONT_LEFT: StandardId = unsafe { StandardId::new_unchecked(0x011) };
pub const CAN_ID_HEIGHT_SENSOR_FRONT_RIGHT: StandardId =
    unsafe { StandardId::new_unchecked(0x012) };
pub const CAN_ID_HEIGHT_SENSOR_RESERVED1: StandardId = unsafe { StandardId::new_unchecked(0x013) };
pub const CAN_ID_HEIGHT_SENSOR_RESERVED2: StandardId = unsafe { StandardId::new_unchecked(0x014) };

#[repr(u8)]
#[allow(dead_code)]
enum HeightSensorState {
    NotPluggedIn = 0x00,
    ModbusError = 0x01,
    Operational = 0x02,
    Unknown = 0xFF,
}

#[embassy_executor::task(pool_size = 4)]
pub async fn height_sensor_task(
    mut uart: usart::Uart<'static, Async>,
    detect: Input<'static>,
    can_id: StandardId,
    mut can_tx: BufferedCanSender,
) {
    loop {
        let (state, height_mm) = if detect.is_high() {
            info!("Height sensor: NotPluggedIn");
            (HeightSensorState::NotPluggedIn, 0u16)
        } else {
            read_height_sensor(&mut uart).await
        };

        let height_le = height_mm.to_le_bytes();
        let frame = Frame::new_data(can_id, &[state as u8, height_le[0], height_le[1]]).unwrap();
        match can_tx.try_write(frame) {
            Ok(()) => {}
            Err(e) => warn!("Height sensor CAN tx error: {:?}", e),
        }

        Timer::after_secs(1).await;
    }
}

async fn read_height_sensor(uart: &mut usart::Uart<'static, Async>) -> (HeightSensorState, u16) {
    let mut mreq = ModbusRequest::new(0x01, ModbusProto::Rtu);
    let mut request: heapless::Vec<u8, 256> = heapless::Vec::new();
    if mreq.generate_get_holdings(0x0101, 1, &mut request).is_err() {
        warn!("Height sensor: failed to build request");
        return (HeightSensorState::ModbusError, 0);
    }

    if let Err(e) = uart.write(&request).await {
        warn!("Height sensor TX error: {}", e);
        return (HeightSensorState::ModbusError, 0);
    }
    uart.flush().await.ok();

    let mut response_buf = [0u8; 32];
    match with_timeout(
        Duration::from_millis(500),
        uart.read_until_idle(&mut response_buf),
    )
    .await
    {
        Ok(Ok(n)) => {
            let mut result: heapless::Vec<u16, 16> = heapless::Vec::new();
            match mreq.parse_u16(&response_buf[..n], &mut result) {
                Ok(()) => {
                    if let Some(&height_mm) = result.first() {
                        info!("Height: {} mm", height_mm);
                        (HeightSensorState::Operational, height_mm)
                    } else {
                        warn!("Height sensor: empty response");
                        (HeightSensorState::ModbusError, 0)
                    }
                }
                Err(_) => {
                    warn!("Height sensor Modbus parse error");
                    (HeightSensorState::ModbusError, 0)
                }
            }
        }
        Ok(Err(e)) => {
            warn!("Height sensor RX error: {}", e);
            (HeightSensorState::ModbusError, 0)
        }
        Err(_) => {
            warn!("Height sensor response timeout");
            (HeightSensorState::ModbusError, 0)
        }
    }
}
