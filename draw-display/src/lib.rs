#![cfg_attr(not(test), no_std)]

mod time;

use core::net::Ipv4Addr;

use embedded_graphics::{
    image::Image,
    mono_font::{
        ascii::{FONT_10X20, FONT_4X6},
        MonoTextStyle, MonoTextStyleBuilder,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle},
    text::{Alignment, Text},
};
use eoi_can_decoder::{
    EoiBattery, EoiCanData, GnssData, GnssDateTime, MpptChannelPower, MpptInfo, ThrottleData,
    VescData,
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
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    pub motor_battery_voltage: DisplayValue<f32>,
    pub motor_battery_current: DisplayValue<f32>,
    pub motor_current: DisplayValue<f32>,
    pub motor_duty_cycle: DisplayValue<f32>,
    pub motor_rpm: DisplayValue<i32>,
    pub motor_fet_temperature: DisplayValue<f32>,
    pub motor_temperature: DisplayValue<f32>,
    pub throttle_value: DisplayValue<f32>,
    pub mppt_panel_info: [DisplayValue<(f32, f32, f32)>; 11], // (Power, Voltage, Current)
    pub charging_disabled: DisplayValue<bool>,
    pub time: DisplayValue<GnssDateTime>,
    pub ip_address: DisplayValue<Ipv4Addr>,
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
                }
                EoiBattery::BatteryUptime(data) => {
                    self.battery_uptime_ms.update(data.uptime_ms);
                }
            },

            EoiCanData::Throttle(throttle) => {
                if let ThrottleData::Status(data) = throttle {
                    self.throttle_value.update(data.value);
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
                if let MpptInfo::MpptChannelPower(MpptChannelPower {
                    mppt_channel,
                    voltage_in,
                    current_in,
                }) = mppt_data.info
                {
                    // we map the mppt channel to the index of the (solar) array
                    // mppt id is what it is, but channel and panel id are indexed from 0
                    if let Some(panel_id) = match (mppt_data.mppt_id, mppt_channel) {
                        (3, 0) => Some(0),
                        (3, 1) => Some(1),
                        (3, 3) => Some(2),
                        (5, 0) => Some(3),
                        (5, 1) => Some(4),
                        (5, 2) => Some(5),
                        (4, 1) => Some(6),
                        (4, 2) => Some(7),
                        (2, 0) => Some(8),
                        (2, 1) => Some(9),
                        (2, 2) => Some(10),
                        _ => None,
                    } {
                        // update the panel info
                        self.mppt_panel_info[panel_id as usize].update((
                            voltage_in * current_in,
                            voltage_in,
                            current_in,
                        ));
                    }
                }
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
    let image_l = Image::new(&bmp, Point::new(0, 0));
    let image_r = Image::new(&bmp, Point::new(800 - 100, 0));
    image_l.draw(&mut display.color_converted())?;
    image_r.draw(&mut display.color_converted())?;

    let black_box: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On.into())
        .background_color(BinaryColor::Off.into())
        .build();

    let normal: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();

    string_helper.clear();
    if let Some(data) = data.time.get() {
        string_helper.clear();
        write!(
            &mut string_helper,
            "{:02}:{:02}:{:02}",
            data.hours, data.minutes, data.seconds
        )
        .unwrap();
    } else {
        string_helper.push_str("Time:N/A").unwrap();
    }
    Text::new(string_helper.as_str(), Point::new(362, 18), normal).draw(display)?;

    // power meter

    Line::new(Point::new(0, 70), Point::new(800, 70))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::Off.into(), 2))
        .draw(display)?;

    if let Some(data) = data.charging_disabled.get() {
        if *data {
            Text::with_alignment(
                "Charging enabled",
                Point::new(400, 50),
                normal,
                Alignment::Center,
            )
            .draw(display)?;
        } else {
            Text::with_alignment(
                "Charging disabled !!!",
                Point::new(400, 50),
                black_box,
                Alignment::Center,
            )
            .draw(display)?;
        }
    }

    Text::with_alignment(
        "Net Power (W)",
        Point::new(300, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    if data.battery_voltage.is_valid()
        && data.battery_current_in.is_valid()
        && data.battery_current_out_motor.is_valid()
        && data.battery_current_out_peripherals.is_valid()
    {
        let voltage = data.battery_voltage.get().unwrap_or(&f32::NAN);
        let current = data.battery_current_in.get().unwrap_or(&f32::NAN)
            + data.battery_current_out_motor.get().unwrap_or(&f32::NAN)
            + data
                .battery_current_out_peripherals
                .get()
                .unwrap_or(&f32::NAN);
        let power = voltage * current;
        write!(&mut string_helper, "{:.1}", power).unwrap();
    } else {
        string_helper.push_str("N/A").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(300, 130),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    Line::new(Point::new(0, 140), Point::new(800, 140))
        .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 2))
        .draw(display)?;

    Text::with_alignment(
        "Speed (km/h)",
        Point::new(100, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();

    if let Some(speed_kmh) = data.speed_kmh.get() {
        if *data.gnss_fix.get().unwrap_or(&false) {
            write!(&mut string_helper, "{:.1}", speed_kmh).unwrap();
        } else {
            string_helper.push_str("No fix").unwrap();
        }
    } else {
        string_helper.push_str("N/A").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(100, 130),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    // state of charge
    Text::with_alignment(
        "State of Charge (%)",
        Point::new(500, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    if let Some(data) = data.battery_state_of_charge.get() {
        write!(&mut string_helper, "{:.1}", data).unwrap();
    } else {
        string_helper.push_str("N/A").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(500, 130),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    Text::with_alignment(
        "Time to empty (Min)",
        Point::new(700, 100),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    string_helper.clear();
    if let Some(data) = data.battery_time_to_empty.get() {
        write!(&mut string_helper, "{}", data).unwrap();
    } else {
        string_helper.push_str("N/A").unwrap();
    }

    Text::with_alignment(
        string_helper.as_str(),
        Point::new(700, 130),
        normal,
        Alignment::Center,
    )
    .draw(display)?;

    // MPPT information

    Text::new(
        "MPPT",
        Point::new(15, 370),
        MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
    )
    .draw(display)?;
    let mut panel_text: String<64> = String::new();
    use core::fmt::Write;
    for (panel, info) in data.mppt_panel_info.iter().enumerate() {
        panel_text.clear();
        if let Some((power, voltage, current)) = info.get() {
            write!(
                &mut panel_text,
                "Panel {:2}: {:.0} W {:.0} V {:.1} A",
                panel + 1,
                power,
                voltage,
                current
            )
            .unwrap();
        } else {
            write!(&mut panel_text, "Panel {:2}: N/A", panel + 1).unwrap();
        }
        Text::new(
            panel_text.as_str(),
            Point::new(15, (panel as i32 * 8) + 390),
            MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
        )
        .draw(display)?;
    }

    // battery information

    let mut battery_offset_y = 200;
    const FONT_10X20_SPACE: i32 = 20;

    Text::new("Battery", Point::new(415, 170), normal).draw(display)?;

    string_helper.clear();
    if data.battery_voltage.is_valid() && data.battery_current_in.is_valid() {
        let voltage = data.battery_voltage.get().unwrap_or(&f32::NAN);
        let current = data.battery_current_in.get().unwrap_or(&f32::NAN);
        let power = voltage * current;
        write!(&mut string_helper, "Input {:.0} W", power).unwrap();
    } else {
        string_helper.push_str("Input N/A").unwrap();
    }

    Text::new(
        string_helper.as_str(),
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    string_helper.clear();
    if data.battery_voltage.is_valid() && data.battery_current_out_motor.is_valid() {
        let voltage = data.battery_voltage.get().unwrap_or(&f32::NAN);
        let current = data.battery_current_out_motor.get().unwrap_or(&f32::NAN);
        let power = voltage * current;
        write!(&mut string_helper, "Output motor {:.0} W", power).unwrap();
    } else {
        string_helper.push_str("Output motor N/A").unwrap();
    }

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        string_helper.as_str(),
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    string_helper.clear();
    if data.battery_voltage.is_valid() && data.battery_current_out_peripherals.is_valid() {
        let voltage = data.battery_voltage.get().unwrap_or(&f32::NAN);
        let current = data
            .battery_current_out_peripherals
            .get()
            .unwrap_or(&f32::NAN);
        let power = voltage * current;
        write!(&mut string_helper, "Output peripherals {:.0} W", power).unwrap();
    } else {
        string_helper.push_str("Output peripherals N/A").unwrap();
    }

    battery_offset_y += FONT_10X20_SPACE;
    Text::new(
        string_helper.as_str(),
        Point::new(415, battery_offset_y),
        normal,
    )
    .draw(display)?;

    // get array of temperatures
    let valid_temperatures = data
        .battery_temperatures
        .iter()
        .filter(|temp| temp.is_valid())
        .map(|temp| temp.get().unwrap_or(&i8::MIN))
        .collect::<heapless::Vec<&i8, 4>>();

    if !valid_temperatures.is_empty() {
        let max_temp = valid_temperatures.iter().copied().max().unwrap_or(&i8::MIN);
        let min_temp = valid_temperatures.iter().copied().min().unwrap_or(&i8::MAX);
        let avg_temp = valid_temperatures
            .iter()
            .copied()
            .map(|&t| t as f32)
            .sum::<f32>()
            / valid_temperatures.len() as f32;

        string_helper.clear();
        write!(&mut string_helper, "Max temperature {} C", max_temp).unwrap();
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            string_helper.as_str(),
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        string_helper.clear();
        write!(&mut string_helper, "Min temperature {} C", min_temp).unwrap();
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            string_helper.as_str(),
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        string_helper.clear();
        write!(&mut string_helper, "Avg temperature {:.0} C", avg_temp).unwrap();
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            string_helper.as_str(),
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;
    } else {
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            "Max temperature N/A",
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            "Min temperature N/A",
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            "Avg temperature N/A",
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;
    }

    // get array of voltages
    let valid_voltages = data
        .battery_cell_voltages
        .iter()
        .filter(|voltage| voltage.is_valid())
        .map(|voltage| voltage.get().unwrap_or(&f32::NAN))
        .collect::<heapless::Vec<&f32, 14>>();

    if !valid_voltages.is_empty() {
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
        write!(&mut string_helper, "Max cell voltage {:.3} V", max_voltage).unwrap();
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            string_helper.as_str(),
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        string_helper.clear();
        write!(&mut string_helper, "Min cell voltage {:.3} V", min_voltage).unwrap();
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            string_helper.as_str(),
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        string_helper.clear();
        write!(&mut string_helper, "Avg cell voltage {:.3} V", avg_voltage).unwrap();
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            string_helper.as_str(),
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;
    } else {
        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            "Max cell voltage N/A",
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            "Min cell voltage N/A",
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;

        battery_offset_y += FONT_10X20_SPACE;
        Text::new(
            "Avg cell voltage N/A",
            Point::new(415, battery_offset_y),
            normal,
        )
        .draw(display)?;
    }

    let mut cell_text: String<64> = String::new();

    for cell in 0..7 {
        cell_text.clear();
        if let Some(cell_voltage) = data.battery_cell_voltages[cell as usize].get() {
            write!(
                &mut cell_text,
                "Cell {:2}: {:1.3} V",
                cell + 1,
                cell_voltage
            )
            .unwrap();
        } else {
            write!(&mut cell_text, "Cell {:2}: N/A", cell + 1).unwrap();
        }

        Text::new(
            cell_text.as_str(),
            Point::new(650, (cell * 8) + 420),
            MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
        )
        .draw(display)?;
    }

    for cell in 7..14 {
        cell_text.clear();
        if let Some(cell_voltage) = data.battery_cell_voltages[cell as usize].get() {
            write!(
                &mut cell_text,
                "Cell {:2}: {:1.3} V",
                cell + 1,
                cell_voltage
            )
            .unwrap();
        } else {
            write!(&mut cell_text, "Cell {:2}: N/A", cell + 1).unwrap();
        }
        Text::new(
            cell_text.as_str(),
            Point::new(730, ((cell - 7) * 8) + 420),
            MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
        )
        .draw(display)?;
    }

    Line::new(Point::new(400, 140), Point::new(400, 480))
        .into_styled(PrimitiveStyle::with_stroke(C::from(BinaryColor::Off), 2))
        .draw(display)?;

    // Create a new window
    let mut motordriver_offset_y = 200;

    Text::new("Motor driver", Point::new(15, 170), normal).draw(display)?;

    string_helper.clear();
    if data.motor_battery_voltage.is_valid() && data.motor_battery_current.is_valid() {
        let voltage = data.motor_battery_voltage.get().unwrap_or(&f32::NAN);
        let current = data.motor_battery_current.get().unwrap_or(&f32::NAN);
        write!(&mut string_helper, "Power {:.0} W", voltage * current).unwrap();
    } else {
        string_helper.push_str("Battery power usage N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.motor_battery_current.get() {
        write!(&mut string_helper, "Battery Current {:.1} A", data).unwrap();
    } else {
        string_helper.push_str("Battery Current N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.motor_current.get() {
        write!(&mut string_helper, "Motor Current {:.1} A", data).unwrap();
    } else {
        string_helper.push_str("Motor Current N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.motor_duty_cycle.get() {
        write!(&mut string_helper, "Duty cycle {:.1}%", data).unwrap();
    } else {
        string_helper.push_str("Duty cycle N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.motor_rpm.get() {
        write!(&mut string_helper, "RPM {}", data).unwrap();
    } else {
        string_helper.push_str("RPM N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.motor_fet_temperature.get() {
        write!(&mut string_helper, "FET temperature {} C", data).unwrap();
    } else {
        string_helper.push_str("FET temperature N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.motor_temperature.get() {
        write!(&mut string_helper, "Motor temperature {} C", data).unwrap();
    } else {
        string_helper.push_str("Motor temperature N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    motordriver_offset_y += FONT_10X20_SPACE;
    string_helper.clear();
    if let Some(data) = data.throttle_value.get() {
        write!(&mut string_helper, "Throttle value {:.1}%", data).unwrap();
    } else {
        string_helper.push_str("Throttle value N/A").unwrap();
    }
    Text::new(
        string_helper.as_str(),
        Point::new(15, motordriver_offset_y),
        normal,
    )
    .draw(display)?;

    string_helper.clear();
    if let Some(data) = data.ip_address.get() {
        write!(&mut string_helper, "Ip address: {}", data).unwrap();
    } else {
        string_helper.push_str("Ip address: N/A").unwrap();
    }

    Text::new(
        string_helper.as_str(),
        Point::new(415, 470),
        MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
    )
    .draw(display)?;

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

    Text::new(
        string_helper.as_str(),
        Point::new(250, 470),
        MonoTextStyle::new(&FONT_4X6, C::from(BinaryColor::Off)),
    )
    .draw(display)?;

    Ok(())
}
