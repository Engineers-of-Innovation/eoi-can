#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    gpio::{Level, Output, Speed},
    peripherals,
    usart::{self, Uart, UartRx},
};
use embassy_time::{Duration, Timer};
use tmc2209::Register;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USART3_4_5_6_LPUART1 => usart::InterruptHandler<peripherals::USART4>;
});

#[embassy_executor::task]
async fn heartbeat_task(mut output: embassy_stm32::gpio::Output<'static>) {
    loop {
        output.toggle();
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn tcm_rx_task(mut rx: UartRx<'static, embassy_stm32::mode::Async>) {
    loop {
        let mut reader = tmc2209::Reader::default();
        let mut buffer = [0u8; 50];
        match rx.read_until_idle(&mut buffer).await {
            Ok(len) => {
                trace!("Received byte from TMC2209: {:?}", &buffer[..len]);
                if let (_, Some(response)) = reader.read_response(&buffer[..len])
                    && response.crc_is_valid()
                {
                    debug!("Parsed TMC2209 response: {:?}", response.data());
                    match response.reg_addr() {
                        Ok(tmc2209::reg::SG_RESULT::ADDRESS) => {
                            let sg_result: tmc2209::reg::SG_RESULT = response.data_u32().into();
                            info!("SG_RESULT: {}", sg_result.get());
                        }
                        _ => {
                            warn!("Received response for unhandled register address");
                        }
                    }
                };
            }
            Err(e) => {
                warn!("Error reading from TMC2209 UART: {:?}", e);
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    info!("Hello World!");

    let status_led = Output::new(p.PD8, Level::High, Speed::Low);
    spawner.spawn(unwrap!(heartbeat_task(status_led)));

    // Initialize stepper motor driver, we use X stage output, which has address 0
    let x_tmc_not_enable = Output::new(p.PB14, Level::Low, Speed::Low);
    let x_tmc_step = Output::new(p.PB13, Level::Low, Speed::Low);
    let x_tmc_direction = Output::new(p.PB12, Level::Low, Speed::Low);
    core::mem::forget(x_tmc_not_enable);
    core::mem::forget(x_tmc_step);
    core::mem::forget(x_tmc_direction);

    // we use uart for controlling TMC2209
    let (mut tx_tmc, rx_tmc) = Uart::new(
        p.USART4,
        p.PC11,
        p.PC10,
        Irqs,
        p.DMA1_CH2,
        p.DMA1_CH3,
        embassy_stm32::usart::Config::default(),
    )
    .unwrap()
    .split();
    spawner.spawn(unwrap!(tcm_rx_task(rx_tmc)));
    Timer::after(Duration::from_millis(100)).await; // wait a bit for uart to settle

    // - Enable UART controlled velocity but in the stopped state.
    // - Enable the `pdn_disable` field necessary for UART comms.
    let mut gconf = tmc2209::reg::GCONF::default();
    let mut vactual = tmc2209::reg::VACTUAL::ENABLED_STOPPED;
    vactual.set(1000);
    gconf.set_pdn_disable(true);
    tmc2209::send_write_request(0, gconf, &mut tx_tmc).unwrap();
    tmc2209::send_write_request(0, vactual, &mut tx_tmc).unwrap();

    loop {
        Timer::after(Duration::from_millis(1000)).await;
        tmc2209::send_read_request::<tmc2209::reg::SG_RESULT, _>(0, &mut tx_tmc).unwrap();
        // tmc2209::send_read_request::<tmc2209::reg::DRV_STATUS, _>(0, &mut tx_tmc).unwrap();
    }
}

// use defmt_or_log::*;
// use embassy_executor::Spawner;
// use embassy_rp::adc::Adc;
// #[cfg(feature = "log")]
// use embassy_rp::bind_interrupts;
// use embassy_rp::gpio::Pull;
// #[cfg(feature = "log")]
// use embassy_rp::peripherals::USB;
// use embassy_rp::pio_programs::uart::{PioUartTx, PioUartTxProgram};
// use embassy_rp::pwm::SetDutyCycle;
// #[cfg(feature = "log")]
// use embassy_rp::usb::Driver;
// use embassy_time::{Duration, Ticker, Timer};
// use micromath::F32Ext;
// use {defmt_rtt as _, panic_probe as _};

// // Wrapper to implement embedded-hal 0.2.x blocking Write trait for PioUartTx
// struct PioUartTxWrapper<'a, PIO: embassy_rp::pio::Instance, const SM: usize>(
//     PioUartTx<'a, PIO, SM>,
// );

// impl<'a, PIO: embassy_rp::pio::Instance, const SM: usize> embedded_hal::blocking::serial::Write<u8>
//     for PioUartTxWrapper<'a, PIO, SM>
// {
//     type Error = core::convert::Infallible;

//     fn bwrite_all(&mut self, buffer: &[u8]) -> Result<(), Self::Error> {
//         for &byte in buffer {
//             // Block on the async write operation
//             embassy_futures::block_on(self.0.write_u8(byte));
//         }
//         Ok(())
//     }

//     fn bflush(&mut self) -> Result<(), Self::Error> {
//         Ok(())
//     }
// }

// bind_interrupts!(struct Irqs {
//     #[cfg(feature = "log")]
//     USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<USB>;
//     ADC_IRQ_FIFO => embassy_rp::adc::InterruptHandler;
//     PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
// });

// #[embassy_executor::main]
// async fn main(spawner: Spawner) {
//     let p = embassy_rp::init(Default::default());
//     #[cfg(feature = "log")]
//     {
//         let driver = Driver::new(p.USB, Irqs);
//         spawner.spawn(logger_task(driver).unwrap());
//     }
//     let heartbeat_led = embassy_rp::gpio::Output::new(p.PIN_5, embassy_rp::gpio::Level::Low);
//     spawner.spawn(heartbeat_task(heartbeat_led).unwrap());

//     for _ in 0..3 {
//         info!(".");
//         Timer::after_secs(1).await;
//     }
//     info!("Staartstuk Controller started!");

//     let cfg: embassy_rp::pwm::Config = Default::default();
//     let pwm_input = embassy_rp::pwm::Pwm::new_input(
//         p.PWM_SLICE6,
//         p.PIN_13,
//         Pull::Up,
//         embassy_rp::pwm::InputMode::RisingEdge,
//         cfg,
//     );
//     spawner.spawn(monitor_flow_task(pwm_input).unwrap());

//     let adc = Adc::new(p.ADC, Irqs, embassy_rp::adc::Config::default());
//     let adc_mcu_temp = embassy_rp::adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR);
//     let adc_water_temp = embassy_rp::adc::Channel::new_pin(p.PIN_27, Pull::None);
//     spawner.spawn(monitor_temperature_task(adc, adc_mcu_temp, adc_water_temp).unwrap());

//     // PWM for water pump control, slowly ramp up duty cycle to limit inrush current
//     let desired_freq_hz = 10_000;
//     let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
//     let divider = 16u8;
//     let period = (clock_freq_hz / (desired_freq_hz * divider as u32)) as u16 - 1;

//     let mut c = embassy_rp::pwm::Config::default();
//     c.top = period;
//     c.divider = divider.into();
//     let mut pwm_output = embassy_rp::pwm::Pwm::new_output_b(p.PWM_SLICE3, p.PIN_23, c.clone());
//     pwm_output.set_duty_cycle_fully_off().unwrap();
//     for pump_duty_cycle in (0..=100).step_by(5) {
//         pwm_output.set_duty_cycle_percent(pump_duty_cycle).unwrap();
//         info!("Pump duty cycle set to {}%", pump_duty_cycle);
//         Timer::after(Duration::from_millis(50)).await;
//     }
//     pwm_output.set_duty_cycle_fully_on().unwrap();

//     // Initialize stepper motor driver TMC2209
//     let tmc_enable = embassy_rp::gpio::Output::new(p.PIN_14, embassy_rp::gpio::Level::Low); // Active low
//     core::mem::forget(tmc_enable);
//     let tmc_step = embassy_rp::gpio::Output::new(p.PIN_7, embassy_rp::gpio::Level::Low);
//     let tmc_direction = embassy_rp::gpio::Output::new(p.PIN_6, embassy_rp::gpio::Level::High);
//     core::mem::forget(tmc_step);
//     core::mem::forget(tmc_direction);

//     // PIO-based UART TX on GPIO15 for TMC2209, because this IO is not connected to a hardware UART (TX)
//     let embassy_rp::pio::Pio {
//         mut common, sm0, ..
//     } = embassy_rp::pio::Pio::new(p.PIO0, Irqs);

//     // Load the PIO UART TX program
//     let tx_program = PioUartTxProgram::new(&mut common);
//     let tmc_tx = PioUartTx::new(115200, &mut common, sm0, p.PIN_15, &tx_program);
//     let mut tmc_tx_wrapper = PioUartTxWrapper(tmc_tx);

//     // Initialise tracking state for the registers we care about.
//     // - Enable UART controlled velocity but in the stopped state.
//     // - Enable the `pdn_disable` field necessary for UART comms.
//     let mut gconf = tmc2209::reg::GCONF::default();
//     let mut vactual = tmc2209::reg::VACTUAL::ENABLED_STOPPED;
//     vactual.set(-1000);
//     gconf.set_pdn_disable(true);
//     tmc2209::send_write_request(0, gconf, &mut tmc_tx_wrapper).unwrap();
//     tmc2209::send_write_request(0, vactual, &mut tmc_tx_wrapper).unwrap();

//     loop {
//         Timer::after(Duration::from_secs(1)).await;
//     }
// }

// #[cfg(feature = "log")]
// #[embassy_executor::task]
// async fn logger_task(driver: Driver<'static, USB>) {
//     embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
// }

// #[embassy_executor::task]
// async fn heartbeat_task(mut output: embassy_rp::gpio::Output<'static>) {
//     loop {
//         output.toggle();
//         Timer::after_secs(1).await;
//     }
// }

// #[embassy_executor::task]
// async fn monitor_flow_task(pwm: embassy_rp::pwm::Pwm<'static>) {
//     // 22.9 Hz (measured) at about 1,88L/min (by datasheet)
//     let mut ticker = Ticker::every(Duration::from_secs(10));
//     loop {
//         let frequency = pwm.counter() as f32 / 10.0;
//         info!("Flow meter input frequency: {:.1} Hz", frequency);
//         let liter_min = frequency / (22.9 / 1.88);
//         info!("Estimated flow: {:.2} L/min", liter_min);
//         pwm.set_counter(0);
//         ticker.next().await;
//     }
// }

// #[embassy_executor::task]
// async fn monitor_temperature_task(
//     mut adc: Adc<'static, embassy_rp::adc::Async>,
//     mut adc_mcu_temp: embassy_rp::adc::Channel<'static>,
//     mut adc_water_temp: embassy_rp::adc::Channel<'static>,
// ) {
//     loop {
//         let mcu_temp = adc.read(&mut adc_mcu_temp).await.unwrap();
//         info!("MCU Temp:     {:.1} °C", mcu_convert_to_celsius(mcu_temp));
//         let water_temp = adc.read(&mut adc_water_temp).await.unwrap();
//         info!(
//             "Water Temp:   {:.1} °C",
//             flow_meter_convert_to_celsius(water_temp)
//         );
//         Timer::after(Duration::from_secs(1)).await;
//     }
// }

// fn mcu_convert_to_celsius(raw_temp: u16) -> f32 {
//     // According to chapter 4.9.5. Temperature Sensor in RP2040 datasheet
//     27.0 - (raw_temp as f32 * 3.3 / 4096.0 - 0.706) / 0.001721
// }

// fn flow_meter_convert_to_celsius(raw_temp: u16) -> f32 {
//     // top resistor 51k plus 1k so 52k total
//     // bottom resistor 50k (at 25°C) ntc (b value 3950)
//     // voltage divider output to adc

//     const R_TOP: f32 = 52000.0; // 52kΩ
//     // const R_NTC_25C: f32 = 50000.0; // 50kΩ at 25°C
//     const R_NTC_25C: f32 = 48000.0; //(calibrated with ice water)
//     const B_VALUE: f32 = 3950.0;
//     const T_25C: f32 = 298.15; // 25°C in Kelvin
//     const ADC_MAX: f32 = 4096.0;

//     // Calculate NTC resistance from voltage divider ratio
//     // ADC/ADC_MAX = R_ntc / (R_top + R_ntc)
//     // R_ntc = (ADC * R_top) / (ADC_MAX - ADC)
//     let r_ntc = (raw_temp as f32 * R_TOP) / (ADC_MAX - raw_temp as f32);

//     // Steinhart-Hart simplified (B parameter equation)
//     // 1/T = 1/T0 + (1/B) * ln(R/R0)
//     let temp_kelvin = 1.0 / (1.0 / T_25C + (1.0 / B_VALUE) * (r_ntc / R_NTC_25C).ln());
//     temp_kelvin - 273.15
// }
