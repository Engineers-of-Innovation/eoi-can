#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::can::{Frame, StandardId};
use embassy_stm32::rcc::{
    AHBPrescaler, APBPrescaler, Hse, HseMode, LsConfig, LseConfig, LseDrive, LseMode, Pll, PllMul,
    PllPreDiv, PllRDiv, PllSource, RtcClockSource, Sysclk,
};
use embassy_stm32::time::Hertz;
use embassy_stm32::{
    bind_interrupts,
    can::{
        Can, CanRx, CanTx, Fifo, Rx0InterruptHandler, Rx1InterruptHandler, SceInterruptHandler,
        TxInterruptHandler, filter::Mask32,
    },
    gpio::{Input, Level, Output, Pull, Speed},
    i2c::{self, Master},
    mode::Async,
    peripherals::{CAN1, I2C2, UART4, UART5, USART2, USART3},
    usart,
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer, with_timeout};
use rmodbus::{client::ModbusRequest, ModbusProto};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

fn clock_config() -> embassy_stm32::Config {
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(16_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,  // 16 MHz HSE
        prediv: PllPreDiv::DIV1, // /1 → 16 MHz
        mul: PllMul::MUL10,      // ×10 → 160 MHz VCO
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2), // /2 → 80 MHz SYSCLK
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.apb2_pre = APBPrescaler::DIV1;
    config.rcc.ls = LsConfig {
        rtc: RtcClockSource::LSE,
        lsi: false,
        lse: Some(LseConfig {
            frequency: Hertz(32_768),
            mode: LseMode::Oscillator(LseDrive::MediumHigh),
        }),
    };
    config
}

bind_interrupts!(struct Irqs {
    CAN1_TX  => TxInterruptHandler<CAN1>;
    CAN1_RX0 => Rx0InterruptHandler<CAN1>;
    CAN1_RX1 => Rx1InterruptHandler<CAN1>;
    CAN1_SCE => SceInterruptHandler<CAN1>;
    I2C2_EV  => i2c::EventInterruptHandler<I2C2>;
    I2C2_ER  => i2c::ErrorInterruptHandler<I2C2>;
    USART2   => usart::InterruptHandler<USART2>;
    USART3   => usart::InterruptHandler<USART3>;
    UART4    => usart::InterruptHandler<UART4>;
    UART5    => usart::InterruptHandler<UART5>;
});

const CAN_ID_HEIGHT_SENSOR_FRONT_LEFT: StandardId = unsafe { StandardId::new_unchecked(0x011) };
const CAN_ID_HEIGHT_SENSOR_FRONT_RIGHT: StandardId = unsafe { StandardId::new_unchecked(0x012) };
const CAN_ID_HEIGHT_SENSOR_RESERVED1: StandardId = unsafe { StandardId::new_unchecked(0x013) };
const CAN_ID_HEIGHT_SENSOR_RESERVED2: StandardId = unsafe { StandardId::new_unchecked(0x014) };

static CAN_TX: StaticCell<Mutex<NoopRawMutex, CanTx<'static>>> = StaticCell::new();

#[embassy_executor::task]
async fn heartbeat_task(mut output: embassy_stm32::gpio::Output<'static>) {
    loop {
        output.toggle();
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn can_rx_task(mut rx: CanRx<'static>) {
    loop {
        match rx.read().await {
            Ok(envelope) => info!("CAN rx: {:02x}", envelope.frame.data()),
            Err(e) => trace!("CAN rx error: {}", e),
        }
    }
}

#[embassy_executor::task]
async fn temperature_task(
    mut i2c: i2c::I2c<'static, Async, Master>,
    can_tx: &'static Mutex<NoopRawMutex, CanTx<'static>>,
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
            "temperature: {}.{:02} C",
            temp_raw / 100,
            (temp_raw % 100).abs()
        );

        let frame =
            Frame::new_data(StandardId::new(0x100).unwrap(), &temp_raw.to_be_bytes()).unwrap();

        // {
        //     let mut tx = can_tx.lock().await;
        //     match tx.try_write(&frame) {
        //         Ok(_) => {}
        //         Err(e) => trace!("CAN tx error: {}", e),
        //     }
        // }

        Timer::after_secs(1).await;
    }
}

#[repr(u8)]
enum HeightSensorState {
    NotPluggedIn = 0x00,
    ModbusError = 0x01,
    Operational = 0x02,
    Unknown = 0xFF,
}

#[embassy_executor::task(pool_size = 4)]
async fn height_sensor_task(
    mut uart: usart::Uart<'static, Async>,
    detect: Input<'static>,
    can_id: StandardId,
    can_tx: &'static Mutex<NoopRawMutex, CanTx<'static>>,
) {

    loop {
        let (state, height_mm) = if detect.is_high() {
            info!("Height sensor: NotPluggedIn");
            (HeightSensorState::NotPluggedIn, 0u16)
        } else {
            read_height_sensor(&mut uart).await
        };

        let height_le = height_mm.to_le_bytes();
        let frame = Frame::new_data(
            can_id,
            &[state as u8, height_le[0], height_le[1]],
        )
        .unwrap();

        // {
        //     let mut tx = can_tx.lock().await;
        //     match tx.try_write(&frame) {
        //         Ok(_) => {}
        //         Err(e) => trace!("CAN tx error: {}", e),
        //     }
        // }

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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    info!("Hello World!");

    let status_led = Output::new(p.PC1, Level::High, Speed::Low);
    spawner.spawn(unwrap!(heartbeat_task(status_led)));

    let can_standby = Output::new(p.PB7, Level::Low, Speed::Low);
    core::mem::forget(can_standby); // keep the pin alive so it doesn't get deinitialized and go into standby mode

    let mut can = Can::new(p.CAN1, p.PB8, p.PB9, Irqs);
    can.modify_filters()
        .enable_bank(0, Fifo::Fifo0, Mask32::accept_all());
    can.modify_config().set_bitrate(1_000_000);
    can.enable().await;

    let (tx, rx) = can.split();
    let can_tx = CAN_TX.init(Mutex::new(tx));

    core::mem::forget(can); // keep the CAN peripheral alive so it doesn't get deinitialized

    spawner.spawn(unwrap!(can_rx_task(rx)));

    let i2c = i2c::I2c::new(
        p.I2C2,
        p.PB10, // SCL
        p.PB11, // SDA
        Irqs,
        p.DMA1_CH4,
        p.DMA1_CH5,
        Default::default(),
    );

    spawner.spawn(unwrap!(temperature_task(i2c, can_tx)));

    let mut uart_config = usart::Config::default();
    uart_config.baudrate = 9600;
    uart_config.parity = usart::Parity::ParityNone;
    uart_config.stop_bits = usart::StopBits::STOP2;
    uart_config.data_bits = usart::DataBits::DataBits8;

    let uart = usart::Uart::new_with_de(
        p.USART2,
        p.PA3,      // RX
        p.PA2,      // TX
        Irqs,
        p.PA1,      // DE (RS-485 direction)
        p.DMA1_CH7, // TX DMA
        p.DMA1_CH6, // RX DMA
        uart_config,
    )
    .unwrap();

    // HeightSensorFrontLeft — USART2
    let height_detect = Input::new(p.PA0, Pull::Down);
    spawner.spawn(unwrap!(height_sensor_task(uart, height_detect, CAN_ID_HEIGHT_SENSOR_FRONT_LEFT, can_tx)));

    // HeightSensorFrontRight — USART3
    let uart3 = usart::Uart::new_with_de(
        p.USART3,
        p.PC5,      // RX
        p.PC4,      // TX
        Irqs,
        p.PB1,      // DE (RS-485 direction)
        p.DMA1_CH2, // TX DMA
        p.DMA1_CH3, // RX DMA
        uart_config,
    )
    .unwrap();
    let height_detect3 = Input::new(p.PB2, Pull::Down);
    spawner.spawn(unwrap!(height_sensor_task(uart3, height_detect3, CAN_ID_HEIGHT_SENSOR_FRONT_RIGHT, can_tx)));

    // HeightSensorReserved1 — UART4
    let uart4 = usart::Uart::new_with_de(
        p.UART4,
        p.PC11,     // RX
        p.PC10,     // TX
        Irqs,
        p.PA15,     // DE (RS-485 direction)
        p.DMA2_CH3, // TX DMA
        p.DMA2_CH5, // RX DMA
        uart_config,
    )
    .unwrap();
    let height_detect4 = Input::new(p.PA12, Pull::Down);
    spawner.spawn(unwrap!(height_sensor_task(uart4, height_detect4, CAN_ID_HEIGHT_SENSOR_RESERVED1, can_tx)));

    // HeightSensorReserved2 — UART5
    let uart5 = usart::Uart::new_with_de(
        p.UART5,
        p.PD2,      // RX
        p.PC12,     // TX
        Irqs,
        p.PB4,      // DE (RS-485 direction)
        p.DMA2_CH1, // TX DMA
        p.DMA2_CH2, // RX DMA
        uart_config,
    )
    .unwrap();
    let height_detect5 = Input::new(p.PB5, Pull::Down);
    spawner.spawn(unwrap!(height_sensor_task(uart5, height_detect5, CAN_ID_HEIGHT_SENSOR_RESERVED2, can_tx)));
}
