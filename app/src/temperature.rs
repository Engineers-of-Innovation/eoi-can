use defmt::*;
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::i2c;
use embassy_stm32::mode::Async;
use embassy_time::{Duration, Timer, with_timeout};

pub const CAN_ID_TEMPERATURE_HEIGHT_SENSORS: StandardId =
    unsafe { StandardId::new_unchecked(0x210) };
pub const CAN_ID_TEMPERATURE_RUDDER_CONTROLLER: StandardId =
    unsafe { StandardId::new_unchecked(0x211) };

#[embassy_executor::task]
pub async fn temperature_task(
    mut i2c: i2c::I2c<'static, Async, i2c::Master>,
    can_id: StandardId,
    mut can_tx: BufferedCanSender,
) {
    const ADDR: u8 = 0x3F;
    const REG_SOFT_RESET: u8 = 0x0C;
    const REG_CTRL: u8 = 0x04;
    const REG_DATA_T_L: u8 = 0x06;
    // CTRL: BDU=1 (bit6) | AVG=00 (bit4) | IF_ADD_INC=1 (bit3) | FREERUN=1 (bit2)
    const CTRL_VAL: u8 = 0x4C;

    // Software reset
    i2c.write(ADDR, &[REG_SOFT_RESET, 0x02]).await.ok(); // SWRESET = 1
    i2c.write(ADDR, &[REG_SOFT_RESET, 0x00]).await.ok(); // SWRESET = 0
    Timer::after_millis(5).await;

    // Configure continuous measurement mode
    i2c.write(ADDR, &[REG_CTRL, CTRL_VAL]).await.ok();

    loop {
        let mut buf = [0u8; 2];
        match with_timeout(
            Duration::from_millis(100),
            i2c.write_read(ADDR, &[REG_DATA_T_L], &mut buf),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!("WSEN-TIDS: read I2C error: {}", e);
                Timer::after_secs(1).await;
                continue;
            }
            Err(_) => {
                warn!("WSEN-TIDS: read I2C timeout");
                Timer::after_secs(1).await;
                continue;
            }
        }
        let temp_raw = i16::from_le_bytes(buf);

        // temp_raw is in centidegrees (0.01 °C per LSB): 2500 = 25.00 °C
        info!(
            "PCB Temperature: {}.{:02} C",
            temp_raw / 100,
            (temp_raw % 100).abs()
        );

        let frame = Frame::new_data(can_id, &temp_raw.to_be_bytes()).unwrap();
        match can_tx.try_write(frame) {
            Ok(()) => {}
            Err(e) => warn!("Temperature CAN tx error: {:?}", e),
        }
        Timer::after_secs(1).await;
    }
}
