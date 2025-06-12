#![cfg_attr(not(test), no_std)]

mod time;

use core::net::Ipv4Addr;

use embedded_graphics::{
    image::Image,
    mono_font::{
        ascii::{FONT_10X20, FONT_4X6, FONT_6X10},
        MonoTextStyle, MonoTextStyleBuilder,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use eoi_can_decoder::{
    BatteryState, ChargeState, DischargeState, EoiBattery, EoiCanData, GnssData, GnssDateTime,
    MpptChannel, MpptInfo, ThrottleData, ThrottleErrors, VescData,
};
use heapless::String;
use time::{Duration, Instant};
use tinybmp::Bmp; // Import EoICanData from the appropriate module

const DISPLAY_VALUE_TIMEOUT: Duration = Duration::from_secs(5);

mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

#[derive(Debug)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct DisplayValue<T> {
    value: Option<T>,
    last_updated: Instant,
}

impl<T> DisplayValue<T> {
    pub fn update(&mut self, value: T) {
        self.value = Some(value);
        self.last_updated = Instant::now();
    }

    pub fn is_valid(&self) -> bool {
        self.value.is_some() && self.last_updated.elapsed() < DISPLAY_VALUE_TIMEOUT
    }

    pub fn get(&self) -> Option<&T> {
        if self.is_valid() {
            self.value.as_ref()
        } else {
            None
        }
    }
}

impl<T> Default for DisplayValue<T> {
    fn default() -> Self {
        Self {
            value: None,
            last_updated: Instant::now(), // We need to set something as initial value, will be updated when first value is set
        }
    }
}

#[derive(Debug, Default)]
pub struct DisplayData {
    pub speed_kmh: DisplayValue<f32>,
    pub gnss_fix: DisplayValue<bool>,
    pub battery_state_of_charge: DisplayValue<f32>,
    pub battery_time_to_empty: DisplayValue<u16>,
    pub battery_cell_voltages: [DisplayValue<f32>; 14],
    pub battery_current_pack: DisplayValue<f32>,
    pub battery_current_in: DisplayValue<f32>,
    pub battery_current_out_motor: DisplayValue<f32>,
    pub battery_current_out_peripherals: DisplayValue<f32>,
    pub battery_voltage: DisplayValue<f32>,
    pub battery_temperatures: [DisplayValue<i8>; 4],
    pub battery_uptime_ms: DisplayValue<u32>,
    pub battery_error_flags: DisplayValue<u32>,
    pub battery_balancing_status: DisplayValue<u16>,
    pub battery_state: DisplayValue<BatteryState>,
    pub battery_charge_state: DisplayValue<ChargeState>,
    pub battery_discharge_state: DisplayValue<DischargeState>,
    pub motor_battery_voltage: DisplayValue<f32>,
    pub motor_battery_current: DisplayValue<f32>,
    pub motor_current: DisplayValue<f32>,
    pub motor_duty_cycle: DisplayValue<f32>,
    pub motor_rpm: DisplayValue<i32>,
    pub motor_fet_temperature: DisplayValue<f32>,
    pub motor_temperature: DisplayValue<f32>,
    pub throttle_value: DisplayValue<f32>,
    pub throttle_errors: DisplayValue<ThrottleErrors>,
    pub mppt_panel_info: [DisplayValue<(f32, f32, f32)>; 11], // (Power, Voltage, Current)
    pub charging_disabled: DisplayValue<bool>,
    pub time: DisplayValue<GnssDateTime>,
    pub ip_address: DisplayValue<Ipv4Addr>,
    pub display_state_of_charge: DisplayValue<f32>,
    pub display_is_charging: DisplayValue<bool>,
}

impl DisplayData {
    pub fn ingest_eoi_can_data(&mut self, data: EoiCanData) {
        match data {
            EoiCanData::EoiBattery(eoi_battery) => match eoi_battery {
                EoiBattery::ChargeAndDischargeCurrent(data) => {
                    self.battery_current_in.update(data.charge_current);
                    self.battery_current_out_motor
                        .update(data.discharge_current);
                }
                EoiBattery::SocErrorFlagsAndBalancing(data) => {
                    self.battery_state_of_charge.update(data.state_of_charge);
                    self.battery_error_flags.update(data.error_flags);
                    self.battery_balancing_status.update(data.balancing_status);
                }
                EoiBattery::PackAndPerriCurrent(data) => {
                    self.battery_current_out_peripherals
                        .update(data.perri_current);
                    self.battery_current_pack.update(data.pack_current);
                }
                EoiBattery::CellVoltages1_4(data) => {
                    self.update_cell_voltages(0, data.cell_voltage.as_slice());
                }
                EoiBattery::CellVoltages5_8(data) => {
                    self.update_cell_voltages(4, data.cell_voltage.as_slice());
                }
                EoiBattery::CellVoltages9_12(data) => {
                    self.update_cell_voltages(8, data.cell_voltage.as_slice());
                }
                EoiBattery::CellVoltages13_14PackAndStack(data) => {
                    self.update_cell_voltages(12, data.cell_voltage.as_slice());
                    self.battery_voltage.update(data.pack_voltage);
                }
                EoiBattery::TemperaturesAndStates(data) => {
                    for (index, value) in data.temperatures.iter().enumerate() {
                        self.battery_temperatures[index].update(*value);
                    }
                    self.battery_state.update(data.battery_state);
                    self.battery_charge_state.update(data.charge_state);
                    self.battery_discharge_state.update(data.discharge_state);
                }
                EoiBattery::BatteryUptime(data) => {
                    self.battery_uptime_ms.update(data.uptime_ms);
                }
            },

            EoiCanData::Throttle(throttle) => {
                if let ThrottleData::Status(data) = throttle {
                    self.throttle_value.update(data.value);
                    self.throttle_errors.update(data.error);
                }
            }

            EoiCanData::Vesc(vesc) => match vesc {
                VescData::StatusMessage1 {
                    rpm,
                    total_current,
                    duty_cycle,
                } => {
                    self.motor_rpm.update(rpm);
                    self.motor_current.update(total_current);
                    self.motor_duty_cycle.update(duty_cycle);
                }
                VescData::StatusMessage4 {
                    fet_temp,
                    motor_temp,
                    total_input_current,
                    current_pid_position: _,
                } => {
                    self.motor_battery_current.update(total_input_current);
                    self.motor_fet_temperature.update(fet_temp);
                    self.motor_temperature.update(motor_temp);
                }
                VescData::StatusMessage5 {
                    input_voltage,
                    tachometer: _,
                } => {
                    self.motor_battery_voltage.update(input_voltage);
                }
                _ => {}
            },
            EoiCanData::Mppt(mppt_data) => {
                let (panel_id, channel_power) = match mppt_data {
                    eoi_can_decoder::MpptData::Id2(MpptInfo::Channel1(MpptChannel::Power(
                        power,
                    ))) => (0, power),
                    eoi_can_decoder::MpptData::Id2(MpptInfo::Channel2(MpptChannel::Power(
                        power,
                    ))) => (1, power),
                    eoi_can_decoder::MpptData::Id2(MpptInfo::Channel3(MpptChannel::Power(
                        power,
                    ))) => (2, power),

                    eoi_can_decoder::MpptData::Id5(MpptInfo::Channel0(MpptChannel::Power(
                        power,
                    ))) => (3, power),
                    eoi_can_decoder::MpptData::Id5(MpptInfo::Channel1(MpptChannel::Power(
                        power,
                    ))) => (4, power),
                    eoi_can_decoder::MpptData::Id5(MpptInfo::Channel2(MpptChannel::Power(
                        power,
                    ))) => (5, power),

                    eoi_can_decoder::MpptData::Id4(MpptInfo::Channel1(MpptChannel::Power(
                        power,
                    ))) => (6, power),
                    eoi_can_decoder::MpptData::Id4(MpptInfo::Channel2(MpptChannel::Power(
                        power,
                    ))) => (7, power),

                    eoi_can_decoder::MpptData::Id6(MpptInfo::Channel0(MpptChannel::Power(
                        power,
                    ))) => (8, power),
                    eoi_can_decoder::MpptData::Id6(MpptInfo::Channel1(MpptChannel::Power(
                        power,
                    ))) => (9, power),
                    eoi_can_decoder::MpptData::Id6(MpptInfo::Channel2(MpptChannel::Power(
                        power,
                    ))) => (10, power),

                    _ => return, // not used mppt id or channel
                };

                // update the panel info
                self.mppt_panel_info[panel_id].update((
                    channel_power.voltage_in * channel_power.current_in,
                    channel_power.voltage_in,
                    channel_power.current_in,
                ));
            }
            EoiCanData::Gnss(gnss) => match gnss {
                GnssData::GnssSpeedAndHeading(speed_kmh, _) => {
                    self.speed_kmh.update(speed_kmh);
                }
                GnssData::GnssDateTime(data) => self.time.update(data),
                GnssData::GnssStatus(data) => {
                    self.gnss_fix.update(data.fix != 0);
                }
                GnssData::GnssLatitude(_) => {}
                GnssData::GnssLongitude(_) => {}
            },
        }
    }

    pub fn update_cell_voltages(&mut self, offset: usize, values: &[f32]) {
        for (index, value) in values.iter().enumerate() {
            self.battery_cell_voltages[offset + index].update(*value);
        }
    }
}

pub fn draw_display<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    display.clear(BinaryColor::On.into())?;
    let mut string_helper: String<64> = String::new();

    let bmp: Bmp<BinaryColor> =
        Bmp::from_slice(include_bytes!("../eoi-logo-mark--monochrome-black.bmp")).unwrap();
    Image::new(&bmp, Point::new(800 - 70, 0)).draw(&mut display.color_converted())?;

    let font_normal_inverted: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On.into())
        .background_color(BinaryColor::Off.into())
        .build();

    let font_normal: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();

    let font_normal_header: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .underline()
        .build();
    const FONT_NORMAL_SPACE: i32 = 20;

    let font_small: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();

    let font_small_inverted: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On.into())
        .background_color(BinaryColor::Off.into())
        .build();
    const FONT_SMALL_SPACE: i32 = 10;

    let font_tiny: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_4X6)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();
    const _FONT_TINY_SPACE: i32 = 8;

    const MOTOR_DRIVER_AND_BATTERY_OFFSET_START: i32 = 160;

    string_helper.clear();
    if let Some(data) = data.time.get() {
        string_helper.clear();
        write!(
            &mut string_helper,
            "Time: {:02}:{:02}:{:02}",
            data.hours, data.minutes, data.seconds
        )
        .unwrap();
    } else {
        string_helper.push_str("Time: N/A").unwrap();
    }
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(400, FONT_NORMAL_SPACE * 2),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    // TODO: implement start of race
    Text::with_alignment(
        "Since Race Start: N/A",
        Point::new(400, FONT_NORMAL_SPACE * 3),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    write!(&mut string_helper, "Throttle Errors: ").unwrap();
    let mut throttle_has_error = false;
    if data.throttle_errors.is_valid() {
        let default_error = ThrottleErrors::default();
        let error = data.throttle_errors.get().unwrap_or(&default_error);
        if error.has_error() {
            throttle_has_error = true;
        }
        write!(&mut string_helper, "{}", error).unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, FONT_NORMAL_SPACE),
        if throttle_has_error {
            font_normal_inverted
        } else {
            font_normal
        },
    )
    .draw(display)?;

    Line::new(Point::new(0, 70), Point::new(800, 70))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::Off.into(), 2))
        .draw(display)?;

    if let Some(data) = data.charging_disabled.get() {
        if *data {
            Text::with_alignment(
                "Charging enabled",
                Point::new(400, 50),
                font_normal,
                Alignment::Center,
            )
            .draw(display)?;
        } else {
            Text::with_alignment(
                "Charging disabled !!!",
                Point::new(400, 50),
                font_normal_inverted,
                Alignment::Center,
            )
            .draw(display)?;
        }
    }

    Text::with_alignment(
        "Net Power",
        Point::new(300, 100),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    let voltage = data.battery_voltage.get().unwrap_or(&f32::NAN);
    let current = data.battery_current_in.get().unwrap_or(&f32::NAN)
        + data.battery_current_out_motor.get().unwrap_or(&f32::NAN)
        + data
            .battery_current_out_peripherals
            .get()
            .unwrap_or(&f32::NAN);
    let power = voltage * current;
    write!(&mut string_helper, "{:.1} W", power).unwrap();

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(300, 130),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    Line::new(Point::new(0, 140), Point::new(800, 140))
        .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 2))
        .draw(display)?;

    Text::with_alignment(
        "Speed",
        Point::new(100, 100),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();

    if *data.gnss_fix.get().unwrap_or(&true) {
        write!(
            &mut string_helper,
            "{:2.1} km/h",
            data.speed_kmh.get().unwrap_or(&f32::NAN)
        )
        .unwrap();
    } else {
        string_helper.push_str("No fix").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(100, 130),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    // state of charge
    Text::with_alignment(
        "State of Charge",
        Point::new(500, 100),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:3.1} %",
        data.battery_state_of_charge.get().unwrap_or(&f32::NAN)
    )
    .unwrap();

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(500, 130),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment(
        "Time to empty",
        Point::new(700, 100),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();

    write!(
        &mut string_helper,
        "{:3} Min",
        data.battery_time_to_empty
            .get()
            .map_or(f32::NAN, |&i| i as f32)
    )
    .unwrap();

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(700, 130),
        font_normal,
        Alignment::Center,
    )
    .draw(display)?;

    // Solar panels information
    Text::new(
        "Solar Panels and MPPT",
        Point::new(15, 360),
        font_normal_header,
    )
    .draw(display)?;
    use core::fmt::Write;
    for (panel, info) in data.mppt_panel_info.iter().enumerate() {
        string_helper.clear();
        if let Some((power, voltage, current)) = info.get() {
            write!(
                &mut string_helper,
                "{:2}: {:4.0} W {:3.0} V {:4.1} A",
                panel + 1,
                power,
                voltage,
                current
            )
            .unwrap();
        } else {
            write!(&mut string_helper, "{:2}: N/A", panel + 1).unwrap();
        }
        Text::new(
            string_helper.as_str(),
            Point::new(15, (panel as i32 * FONT_SMALL_SPACE) + 375),
            font_small,
        )
        .draw(display)?;
    }

    for panel in 0..data.mppt_panel_info.len() {
        let bottom_left = Point::new(220, (panel as i32 * FONT_SMALL_SPACE) + 375 + 2);
        let panel_box = Point::new(150, -FONT_SMALL_SPACE);
        // draw outline of cell voltages boxes
        Rectangle::with_corners(bottom_left, bottom_left + panel_box)
            .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 1))
            .draw(display)?;
        if let Some((power, _, _)) = data.mppt_panel_info[panel].get() {
            let panel_level =
                Point::new(scale_to_range(0.0, 150.0, *power, 150), -FONT_SMALL_SPACE);
            // draw infill for level indication
            Rectangle::with_corners(bottom_left, bottom_left + panel_level)
                .into_styled(PrimitiveStyle::with_fill(C::from(BinaryColor::Off)))
                .draw(display)?;
        }
    }

    // battery information

    let mut battery_offset_y = MOTOR_DRIVER_AND_BATTERY_OFFSET_START;
    let battery_offset_left = 415;
    let battery_offset_right = 790;

    Text::new(
        "Battery",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal_header,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE + 5;

    string_helper.clear();
    let input_power = data.battery_voltage.get().unwrap_or(&f32::NAN)
        * data.battery_current_in.get().unwrap_or(&f32::NAN);
    write!(&mut string_helper, "{:6.0} W", input_power).unwrap();

    Text::new(
        "Input",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        font_normal,
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    let motor_power = data.battery_voltage.get().unwrap_or(&f32::NAN)
        * data.battery_current_out_motor.get().unwrap_or(&f32::NAN);
    write!(&mut string_helper, "{:6.0} W", motor_power).unwrap();

    Text::new(
        "Output motor",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        font_normal,
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    let peripherals_power = data.battery_voltage.get().unwrap_or(&f32::NAN)
        * data
            .battery_current_out_peripherals
            .get()
            .unwrap_or(&f32::NAN);
    write!(&mut string_helper, "{:6.0} W", peripherals_power).unwrap();

    Text::new(
        "Output peripherals",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        font_normal,
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    // get array of temperatures
    let valid_temperatures = data
        .battery_temperatures
        .iter()
        .filter(|temp| temp.is_valid())
        .map(|temp| temp.get().unwrap_or(&i8::MIN))
        .collect::<heapless::Vec<&i8, 4>>();

    let max_temp = valid_temperatures
        .iter()
        .copied()
        .max()
        .map_or(f32::NAN, |&i| i as f32);
    let min_temp = valid_temperatures
        .iter()
        .copied()
        .min()
        .map_or(f32::NAN, |&i| i as f32);
    let avg_temp = valid_temperatures
        .iter()
        .copied()
        .map(|&t| t as f32)
        .sum::<f32>()
        / valid_temperatures.len() as f32;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:2.0}/{:2.0}/{:2.0} C",
        min_temp, max_temp, avg_temp
    )
    .unwrap();

    Text::new(
        "Temperature min/max/avg",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        font_normal,
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    // get array of voltages
    let valid_voltages = data
        .battery_cell_voltages
        .iter()
        .filter(|voltage| voltage.is_valid())
        .map(|voltage| voltage.get().unwrap_or(&f32::NAN))
        .collect::<heapless::Vec<&f32, 14>>();

    let max_voltage = valid_voltages
        .iter()
        .copied()
        .cloned()
        .fold(f32::NAN, f32::max);
    let min_voltage = valid_voltages
        .iter()
        .copied()
        .cloned()
        .fold(f32::NAN, f32::min);
    let avg_voltage =
        valid_voltages.iter().copied().cloned().sum::<f32>() / valid_voltages.len() as f32;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:1.3}/{:1.3} V",
        min_voltage, max_voltage,
    )
    .unwrap();
    Text::new(
        "Cell voltage min/max",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        font_normal,
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:1.3}/{:1.3} V",
        avg_voltage,
        (max_voltage - min_voltage)
    )
    .unwrap();
    Text::new(
        "Cell voltage avg/diff",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        font_normal,
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:?}  ",
        *data.battery_state.get().unwrap_or(&BatteryState::Unknown)
    )
    .unwrap();
    Text::new(
        "Battery State",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        if matches!(
            *data.battery_state.get().unwrap_or(&BatteryState::Unknown),
            BatteryState::On
        ) {
            font_normal
        } else {
            font_normal_inverted
        },
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:?}  ",
        *data
            .battery_charge_state
            .get()
            .unwrap_or(&ChargeState::Unknown)
    )
    .unwrap();

    Text::new(
        "Charge State",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        if matches!(
            *data
                .battery_charge_state
                .get()
                .unwrap_or(&ChargeState::Unknown),
            ChargeState::FetOn
        ) {
            font_normal
        } else {
            font_normal_inverted
        },
        Alignment::Right,
    )
    .draw(display)?;
    battery_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:?}  ",
        *data
            .battery_discharge_state
            .get()
            .unwrap_or(&DischargeState::Unknown)
    )
    .unwrap();

    Text::new(
        "Discharge State",
        Point::new(battery_offset_left, battery_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::with_alignment(
        string_helper.as_str(),
        Point::new(battery_offset_right, battery_offset_y),
        if matches!(
            *data
                .battery_discharge_state
                .get()
                .unwrap_or(&DischargeState::Unknown),
            DischargeState::On
        ) {
            font_normal
        } else {
            font_normal_inverted
        },
        Alignment::Right,
    )
    .draw(display)?;

    // Cell voltages
    const CELL_VOLTAGES_HEIGTH: i32 = 80;
    const CELL_VOLTAGES_WIDTH: i32 = 10;
    const CELL_SPACING: i32 = 28;

    for cell in 0..data.battery_cell_voltages.len() {
        let bottom_left = Point::new(battery_offset_left + cell as i32 * CELL_SPACING, 480 - 10);
        let cell_box = Point::new(CELL_VOLTAGES_WIDTH, -CELL_VOLTAGES_HEIGTH);
        let text_top_left = bottom_left + cell_box.y_axis() + Point::new(1, -3);
        // draw outline of cell voltages boxes
        Rectangle::with_corners(bottom_left, bottom_left + cell_box)
            .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 1))
            .draw(display)?;
        let cell_level = scale_to_range(
            2.5,
            4.2,
            *data.battery_cell_voltages[cell].get().unwrap_or(&f32::NAN),
            CELL_VOLTAGES_HEIGTH,
        );
        // draw infill for level indication
        let cell_level = Point::new(CELL_VOLTAGES_WIDTH, -cell_level);
        Rectangle::with_corners(bottom_left, bottom_left + cell_level)
            .into_styled(PrimitiveStyle::with_fill(C::from(BinaryColor::Off)))
            .draw(display)?;
        // set cell id on top
        string_helper.clear();
        write!(&mut string_helper, "{:2}", cell + 1).unwrap();
        Text::new(string_helper.as_str(), text_top_left, font_tiny).draw(display)?;
    }

    Line::new(Point::new(400, 140), Point::new(400, 480))
        .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 2))
        .draw(display)?;

    // Create a new window
    let mut motor_driver_offset_y = MOTOR_DRIVER_AND_BATTERY_OFFSET_START;
    let motor_driver_offset_left = 15;
    let motor_driver_offset_right = 250;

    Text::new(
        "Motor driver",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal_header,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE + 5;

    let motor_battery_power = data.motor_battery_voltage.get().unwrap_or(&f32::NAN)
        * data.motor_battery_current.get().unwrap_or(&f32::NAN);

    string_helper.clear();
    write!(&mut string_helper, "{:6.0} W", motor_battery_power).unwrap();
    Text::new(
        "Battery power usage",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.1} A",
        data.motor_battery_current.get().unwrap_or(&f32::NAN)
    )
    .unwrap();
    Text::new(
        "Battery Current",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.1} A",
        data.motor_current.get().unwrap_or(&f32::NAN)
    )
    .unwrap();
    Text::new(
        "Motor Current",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.1} %",
        data.motor_duty_cycle.get().unwrap_or(&f32::NAN)
    )
    .unwrap();
    Text::new(
        "Duty cycle",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.0}",
        data.motor_rpm.get().map_or(f32::NAN, |&i| i as f32)
    )
    .unwrap();
    Text::new(
        "RPM",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.1} C",
        data.motor_fet_temperature.get().unwrap_or(&f32::NAN)
    )
    .unwrap();
    Text::new(
        "FET temperature",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.1} C",
        data.motor_temperature.get().unwrap_or(&f32::NAN)
    )
    .unwrap();
    Text::new(
        "Motor temperature",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    motor_driver_offset_y += FONT_NORMAL_SPACE;

    string_helper.clear();
    write!(
        &mut string_helper,
        "{:6.1} %",
        data.throttle_value.get().unwrap_or(&f32::NAN)
    )
    .unwrap();
    Text::new(
        "Throttle value",
        Point::new(motor_driver_offset_left, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;
    Text::new(
        string_helper.as_str(),
        Point::new(motor_driver_offset_right, motor_driver_offset_y),
        font_normal,
    )
    .draw(display)?;

    string_helper.clear();
    if let Some(data) = data.ip_address.get() {
        write!(&mut string_helper, "Ip address: {}", data).unwrap();
    } else {
        string_helper.push_str("Ip address: N/A").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(15, 65),
        font_small,
        Alignment::Left,
    )
    .draw(display)?;

    if let Some(charging) = data.display_is_charging.get() {
        string_helper.clear();
        write!(
            &mut string_helper,
            "Display {:3.0}% {}",
            data.display_state_of_charge.get().unwrap_or(&f32::NAN),
            if *charging {
                "Charging"
            } else {
                "Not Charging"
            },
        )
        .unwrap();
        Text::with_alignment(
            string_helper.as_str(),
            Point::new(730, 65),
            if *charging {
                font_small
            } else {
                font_small_inverted
            },
            Alignment::Right,
        )
        .draw(display)?;
    }

    string_helper.clear();

    write!(
        &mut string_helper,
        "Version: {}, Git: {:.8}{}",
        built_info::PKG_VERSION,
        built_info::GIT_COMMIT_HASH.unwrap_or("unknown"),
        if built_info::GIT_DIRTY.unwrap_or(false) {
            "-dirty"
        } else {
            ""
        }
    )
    .unwrap();

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(800 - 10, 478),
        font_tiny,
        Alignment::Right,
    )
    .draw(display)?;

    Ok(())
}

fn scale_to_range(in_min: f32, in_max: f32, input: f32, out_max: i32) -> i32 {
    let corrected_input = if input.is_nan() {
        in_min
    } else {
        input.clamp(in_min, in_max)
    };
    (((corrected_input - in_min) / (in_max - in_min)) * out_max as f32) as i32
}

#[cfg(test)]
mod tests {
    use std::f32;

    use super::*;
    #[test]
    fn scale_cell_voltages() {
        let range_to_scale_to = 100;
        assert_eq!(scale_to_range(2.5, 4.2, 4.2, range_to_scale_to), 100);
        assert_eq!(scale_to_range(2.5, 4.2, 2.5, range_to_scale_to), 0);
        assert_eq!(scale_to_range(2.5, 4.2, 3.35, range_to_scale_to), 50);
        assert_eq!(scale_to_range(2.5, 4.2, f32::NAN, range_to_scale_to), 0);
    }
}
