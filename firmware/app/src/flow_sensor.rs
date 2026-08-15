use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::adc::{Adc, AdcChannel, SampleTime};
use embassy_stm32::can::{BufferedCanSender, Frame, StandardId};
use embassy_stm32::gpio::{AfType, Flex, Pull};
use embassy_stm32::pac::timer::vals;
use embassy_stm32::peripherals::{ADC2, PA0, PA1, PA2, PA3, TIM2, TIM15};
use embassy_stm32::timer::low_level::{SlaveMode, Timer as LowLevelTimer, TriggerSource};
use embassy_stm32::timer::{Ch1, GeneralInstance4Channel, TimerPin};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Ticker};
use libm::logf;
use static_cell::StaticCell;

pub const CAN_ID_FLOW_IN: StandardId = unsafe { StandardId::new_unchecked(0x215) };
pub const CAN_ID_FLOW_OUT: StandardId = unsafe { StandardId::new_unchecked(0x216) };

// Sensor calibration: 22.9 Hz at 1.88 L/min (= 1880 mL/min) per datasheet.
// mL/min = pulses_per_second * 60 * 1880 / 22.9 / 60 = pulses_per_second * 1_880_000 / 22_900.
const ML_PER_MIN_NUM: u32 = 1_880_000;
const ML_PER_MIN_DEN: u32 = 22_900;

// NTC voltage divider for the flow sensors:
// 47 kΩ to VREF on top, NTC to GND on bottom. NTC: 50 kΩ at 25 °C, B = 3950 K.
const FLOW_NTC: NtcParams = NtcParams {
    top_ohms: 47_000.0,
    r0_ohms: 50_000.0,
    t0_k: 298.15,
    b_k: 3950.0,
};
const ADC_MAX: u16 = 4095;
// Returned when the NTC reads as open or shorted, so consumers can flag the sensor as bad
// without seeing an absurd temperature.
const TEMP_INVALID_CDEG: i16 = i16::MIN;

// 1 Hz broadcast.
const TICK_PERIOD_MS: u64 = 1000;

pub type Adc2Mutex = Mutex<CriticalSectionRawMutex, Adc<'static, ADC2>>;
static ADC2_CELL: StaticCell<Adc2Mutex> = StaticCell::new();

pub struct FlowIn {
    pub timer: LowLevelTimer<'static, TIM2>,
    pub ntc: Peri<'static, PA1>,
}

pub struct FlowOut {
    pub timer: LowLevelTimer<'static, TIM15>,
    pub ntc: Peri<'static, PA3>,
}

pub fn init_adc2(adc2: Peri<'static, ADC2>) -> &'static Adc2Mutex {
    ADC2_CELL.init(Mutex::new(Adc::new(adc2)))
}

pub fn flow_in_init(
    tim2: Peri<'static, TIM2>,
    pulse_in: Peri<'static, PA0>,
    ntc_in: Peri<'static, PA1>,
) -> FlowIn {
    let timer = setup_pulse_counter(tim2, pulse_in);
    FlowIn { timer, ntc: ntc_in }
}

pub fn flow_out_init(pulse_out: Peri<'static, PA2>, ntc_out: Peri<'static, PA3>) -> FlowOut {
    // TIM15 is missing from embassy-stm32's `Peripherals` struct on STM32L471 (it shares the
    // TIM1_BRK interrupt with TIM1), so steal it. We never expose another handle to TIM15, so
    // the singleton invariant holds.
    let tim15 = unsafe { embassy_stm32::peripherals::TIM15::steal() };
    let timer = setup_pulse_counter(tim15, pulse_out);
    FlowOut {
        timer,
        ntc: ntc_out,
    }
}

fn setup_pulse_counter<T, P>(
    tim: Peri<'static, T>,
    pin: Peri<'static, P>,
) -> LowLevelTimer<'static, T>
where
    T: GeneralInstance4Channel,
    P: TimerPin<T, Ch1>,
{
    // Hold the pin in alternate-function (timer-input) mode for the lifetime of the program.
    // Flex::drop disconnects, so leak it.
    let af = pin.af_num();
    let mut flex = Flex::new(pin);
    flex.set_as_af_unchecked(af, AfType::input(Pull::None));
    core::mem::forget(flex);

    let timer = LowLevelTimer::new(tim);
    let r = timer.regs_gp16();
    // CC1 channel = input mapped to TI1.
    r.ccmr_input(0)
        .modify(|w| w.set_ccs(0, vals::CcmrInputCcs::TI4));
    // Enable CC1, capture rising edges (CCxNP/CCxP both 0 = rising-only).
    r.ccer().modify(|w| {
        w.set_cce(0, true);
        w.set_ccp(0, false);
    });
    // External clock mode 1: CNT increments on each TI1FP1 edge.
    r.smcr().modify(|w| {
        w.set_sms(SlaveMode::EXT_CLOCK_MODE);
        w.set_ts(TriggerSource::TI1FP1);
    });
    timer.start();
    timer
}

fn read_pulses<T: GeneralInstance4Channel>(
    timer: &LowLevelTimer<'static, T>,
    prev_cnt: &mut u16,
) -> u16 {
    let cnt = timer.regs_gp16().cnt().read().cnt();
    let pulses = cnt.wrapping_sub(*prev_cnt);
    *prev_cnt = cnt;
    pulses
}

fn pulses_to_milliliter_per_minute(pulses_per_s: u16) -> u16 {
    let ml = (pulses_per_s as u32 * ML_PER_MIN_NUM) / ML_PER_MIN_DEN;
    ml.min(u16::MAX as u32) as u16
}

struct NtcParams {
    pub top_ohms: f32,
    pub r0_ohms: f32,
    pub t0_k: f32,
    pub b_k: f32,
}

fn ntc_raw_to_centidegrees(raw: u16, p: &NtcParams) -> i16 {
    if raw == 0 || raw >= ADC_MAX {
        return TEMP_INVALID_CDEG;
    }
    let r_ntc = p.top_ohms * (raw as f32) / ((ADC_MAX - raw) as f32);
    let inv_t = 1.0 / p.t0_k + logf(r_ntc / p.r0_ohms) / p.b_k;
    let t_celsius = 1.0 / inv_t - 273.15;
    let cdeg = t_celsius * 100.0;
    if cdeg <= i16::MIN as f32 + 1.0 {
        i16::MIN + 1
    } else if cdeg >= i16::MAX as f32 {
        i16::MAX
    } else {
        cdeg as i16
    }
}

fn raw_to_centidegrees_flow(raw: u16) -> i16 {
    ntc_raw_to_centidegrees(raw, &FLOW_NTC)
}

async fn read_ntc(adc: &'static Adc2Mutex, pin: &mut impl AdcChannel<ADC2>) -> u16 {
    let mut guard = adc.lock().await;
    guard.blocking_read(pin, SampleTime::CYCLES247_5)
}

async fn broadcast<T: GeneralInstance4Channel>(
    can_id: StandardId,
    timer: &LowLevelTimer<'static, T>,
    prev_cnt: &mut u16,
    adc: &'static Adc2Mutex,
    ntc: &mut impl AdcChannel<ADC2>,
    can_tx: &mut BufferedCanSender,
) {
    let pulses = read_pulses(timer, prev_cnt);
    let raw_adc = read_ntc(adc, ntc).await;
    let mlpm = pulses_to_milliliter_per_minute(pulses);
    let cdeg = raw_to_centidegrees_flow(raw_adc);

    info!(
        "Flow sensor {:#x}: {} mL/min, {} °C (raw ADC {})",
        can_id.as_raw(),
        mlpm,
        cdeg as f32 / 100.0,
        raw_adc
    );

    let mlpm_le = mlpm.to_le_bytes();
    let cdeg_le = cdeg.to_le_bytes();
    let pulses_le = pulses.to_le_bytes();
    let raw_le = raw_adc.to_le_bytes();
    let frame = Frame::new_data(
        can_id,
        &[
            mlpm_le[0],
            mlpm_le[1],
            cdeg_le[0],
            cdeg_le[1],
            pulses_le[0],
            pulses_le[1],
            raw_le[0],
            raw_le[1],
        ],
    )
    .unwrap();
    if let Err(e) = can_tx.try_write(frame) {
        warn!("Flow sensor {:#x} CAN tx error: {:?}", can_id.as_raw(), e);
    }
}

#[embassy_executor::task]
pub async fn flow_in_task(
    timer: LowLevelTimer<'static, TIM2>,
    adc: &'static Adc2Mutex,
    mut ntc: Peri<'static, PA1>,
    mut can_tx: BufferedCanSender,
) {
    let mut prev_cnt: u16 = timer.regs_gp16().cnt().read().cnt();
    let mut ticker = Ticker::every(Duration::from_millis(TICK_PERIOD_MS));
    loop {
        ticker.next().await;
        broadcast(
            CAN_ID_FLOW_IN,
            &timer,
            &mut prev_cnt,
            adc,
            &mut ntc,
            &mut can_tx,
        )
        .await;
    }
}

#[embassy_executor::task]
pub async fn flow_out_task(
    timer: LowLevelTimer<'static, TIM15>,
    adc: &'static Adc2Mutex,
    mut ntc: Peri<'static, PA3>,
    mut can_tx: BufferedCanSender,
) {
    let mut prev_cnt: u16 = timer.regs_gp16().cnt().read().cnt();
    let mut ticker = Ticker::every(Duration::from_millis(TICK_PERIOD_MS));
    loop {
        ticker.next().await;
        broadcast(
            CAN_ID_FLOW_OUT,
            &timer,
            &mut prev_cnt,
            adc,
            &mut ntc,
            &mut can_tx,
        )
        .await;
    }
}
