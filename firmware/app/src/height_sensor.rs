use defmt::*;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::Input;
use embassy_stm32::mode::Async;
use embassy_stm32::usart;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use rmodbus::{ModbusProto, client::ModbusRequest};

static HEIGHT_TICK_FRONT_LEFT: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_TICK_FRONT_RIGHT: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_DONE_FRONT_LEFT: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static HEIGHT_DONE_FRONT_RIGHT: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Target combined staggered poll rate for the two front ultrasonic sensors.
///
/// The sequencer alternates front-left / front-right and only starts the next
/// sensor after the previous read completes, so ultrasonic pings never overlap.
/// When a read finishes within the slot, the sequencer paces to this rate
/// (8 Hz per sensor). If a read takes the full ~[`ULTRASONIC_PING_MS`], the
/// combined rate naturally drops toward 10 Hz.
const HEIGHT_SENSOR_POLL_RATE_HZ: u64 = 16;

/// Typical ultrasonic measurement time after the Modbus request is sent.
/// Also used as the Modbus response timeout.
const ULTRASONIC_PING_MS: u64 = 100;

/// Target slot duration between starting successive sensor reads.
const POLL_SLOT_MICROS: u64 = 1_000_000 / HEIGHT_SENSOR_POLL_RATE_HZ;

/// Per-sensor period at the target combined rate (two slots).
const SENSOR_TICK_PERIOD_MS: u64 = 1000 * 2 / HEIGHT_SENSOR_POLL_RATE_HZ;

fn height_tick_signal(index: u8) -> &'static Signal<CriticalSectionRawMutex, ()> {
    match index {
        0 => &HEIGHT_TICK_FRONT_LEFT,
        1 => &HEIGHT_TICK_FRONT_RIGHT,
        _ => core::panic!("invalid height sensor index"),
    }
}

fn height_done_signal(index: u8) -> &'static Signal<CriticalSectionRawMutex, ()> {
    match index {
        0 => &HEIGHT_DONE_FRONT_LEFT,
        1 => &HEIGHT_DONE_FRONT_RIGHT,
        _ => core::panic!("invalid height sensor index"),
    }
}

pub const CAN_ID_HEIGHT_SENSOR_FRONT_LEFT: StandardId = unsafe { StandardId::new_unchecked(0x011) };
pub const CAN_ID_HEIGHT_SENSOR_FRONT_RIGHT: StandardId =
    unsafe { StandardId::new_unchecked(0x012) };

#[repr(u8)]
#[allow(dead_code)]
enum HeightSensorState {
    NotPluggedIn = 0x00,
    ModbusError = 0x01,
    Operational = 0x02,
    Unknown = 0xFF,
}

/// Alternates front-left / front-right, waiting for each read to finish before
/// starting the next so ultrasonic pings do not interfere.
#[embassy_executor::task]
pub async fn height_sensor_timer_task() {
    let slot = Duration::from_micros(POLL_SLOT_MICROS);
    let mut next: u8 = 0;
    loop {
        let slot_start = Instant::now();
        height_tick_signal(next).signal(());
        height_done_signal(next).wait().await;

        let elapsed = slot_start.elapsed();
        if elapsed < slot {
            Timer::after(slot - elapsed).await;
        }

        next ^= 1;
    }
}

#[embassy_executor::task(pool_size = 2)]
pub async fn height_sensor_task(
    mut uart: usart::Uart<'static, Async>,
    detect: Input<'static>,
    can_id: StandardId,
    mut can_tx: BufferedCanSender,
    tick_index: u8,
) {
    loop {
        height_tick_signal(tick_index).wait().await;
        let iteration_start = Instant::now();

        let (state, height_mm) = if detect.is_high() {
            info!("Height sensor: NotPluggedIn");
            (HeightSensorState::NotPluggedIn, 0u16)
        } else {
            let start = Instant::now();
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
        if elapsed_ms > SENSOR_TICK_PERIOD_MS {
            let missed = (elapsed_ms - 1) / SENSOR_TICK_PERIOD_MS;
            warn!(
                "Height sensor {}: iteration took {} ms, missed {} tick(s)",
                tick_index, elapsed_ms, missed
            );
        }

        // Release the sequencer so the opposite sensor can start its ping.
        height_done_signal(tick_index).signal(());
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

    // Sending the request starts the ultrasonic ping on the sensor.
    if let Err(e) = uart.write(&request).await {
        warn!("Height sensor TX error: {}", e);
        return (HeightSensorState::ModbusError, 0);
    }
    uart.flush().await.ok();

    const MODBUS_RESPONSE_LEN: usize = 7; // slave(1) + func(1) + byte_count(1) + data(2) + crc(2)
    let mut response_buf = [0u8; MODBUS_RESPONSE_LEN];
    match with_timeout(
        Duration::from_millis(ULTRASONIC_PING_MS),
        uart.read(&mut response_buf),
    )
    .await
    {
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
