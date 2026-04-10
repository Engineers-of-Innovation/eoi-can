use defmt::*;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::Input;
use embassy_stm32::mode::Async;
use embassy_stm32::usart;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Ticker, with_timeout};

static HEIGHT_TICK_0: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_TICK_1: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_TICK_2: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_TICK_3: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_SENSOR_UPDATE_RATE_HZ: u64 = 8;

fn height_tick_signal(index: u8) -> &'static Signal<CriticalSectionRawMutex, ()> {
    match index {
        0 => &HEIGHT_TICK_0,
        1 => &HEIGHT_TICK_1,
        2 => &HEIGHT_TICK_2,
        3 => &HEIGHT_TICK_3,
        _ => core::panic!("invalid height sensor index"),
    }
}
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

#[embassy_executor::task]
pub async fn height_sensor_timer_task() {
    let mut ticker = Ticker::every(Duration::from_millis(1000 / HEIGHT_SENSOR_UPDATE_RATE_HZ));
    loop {
        ticker.next().await;
        for i in 0..4u8 {
            height_tick_signal(i).signal(());
        }
    }
}

#[embassy_executor::task(pool_size = 4)]
pub async fn height_sensor_task(
    mut uart: usart::Uart<'static, Async>,
    detect: Input<'static>,
    can_id: StandardId,
    mut can_tx: BufferedCanSender,
    tick_index: u8,
) {
    const TICK_PERIOD_MS: u64 = 100;
    loop {
        height_tick_signal(tick_index).wait().await;
        let iteration_start = embassy_time::Instant::now();

        let (state, height_mm) = if detect.is_high() {
            info!("Height sensor: NotPluggedIn");
            (HeightSensorState::NotPluggedIn, 0u16)
        } else {
            let start = embassy_time::Instant::now();
            let result = read_height_sensor(&mut uart).await;
            let elapsed = start.elapsed();
            info!(
                "Height sensor {}: read took {} ms",
                tick_index,
                elapsed.as_millis()
            );
            result
        };

        let height_le = height_mm.to_le_bytes();
        let frame = Frame::new_data(can_id, &[state as u8, height_le[0], height_le[1]]).unwrap();
        match can_tx.try_write(frame) {
            Ok(()) => {}
            Err(e) => warn!("Height sensor CAN tx error: {:?}", e),
        }

        let elapsed_ms = iteration_start.elapsed().as_millis();
        if elapsed_ms > TICK_PERIOD_MS {
            let missed = (elapsed_ms - 1) / TICK_PERIOD_MS;
            warn!(
                "Height sensor {}: iteration took {} ms, missed {} tick(s)",
                tick_index, elapsed_ms, missed
            );
        }
    }
}

async fn read_height_sensor(uart: &mut usart::Uart<'static, Async>) -> (HeightSensorState, u16) {
    // Drain any stale bytes left in the RX buffer from a previous timed-out read
    let mut drain = [0u8; 16];
    let _ = with_timeout(Duration::from_ticks(1), uart.read(&mut drain)).await;

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

    const MODBUS_RESPONSE_LEN: usize = 7; // slave(1) + func(1) + byte_count(1) + data(2) + crc(2)
    let mut response_buf = [0u8; MODBUS_RESPONSE_LEN];
    match with_timeout(Duration::from_millis(100), uart.read(&mut response_buf)).await {
        Ok(Ok(())) => {
            let mut result: heapless::Vec<u16, 16> = heapless::Vec::new();
            match mreq.parse_u16(&response_buf, &mut result) {
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
