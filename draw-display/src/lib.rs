#![cfg_attr(not(test), no_std)]

mod render;
mod time;

pub use render::dashboard::draw_display;
pub use render::foiling::draw_foiling;
pub use render::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

use core::net::Ipv4Addr;

use eoi_can_decoder::{
    BatteryState, ChargeState, DataLoggerData, DischargeState, EoiBattery, EoiCanData,
    GanMpptPacket, GnssData, GnssDateTime, HeightSensorData, MpptChannel, MpptInfo,
    TemperatureData, ThrottleData, ThrottleErrors, VescData,
};
use mppt_layout::{gan_side_and_position, position_of, MpptKind, Side, GAN_STRAP_COUNT, LAYOUT};

const MPPT_PANEL_COUNT: usize = LAYOUT.len();
use time::{Duration, Instant};

const DISPLAY_VALUE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which screen to draw.
///
/// Both boards run the same decode pipeline into the same [`DisplayData`] and
/// differ only in the layout they render it with, so the choice is one value
/// rather than two code paths: a firmware bin fixes it at compile time, and the
/// simulator and framebuffer take it from `--layout` so a screen can be worked
/// on without flashing anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum Layout {
    /// The helm dashboard: speed, power, temperatures, times.
    #[default]
    Dashboard,
    /// Foiling trim and tuning parameters.
    Foiling,
}

impl Layout {
    /// Every layout, for a front-end to list in its `--help`.
    pub const ALL: [Self; 2] = [Self::Dashboard, Self::Foiling];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Foiling => "foiling",
        }
    }

    pub fn draw<D, C>(self, display: &mut D, data: &DisplayData) -> Result<(), D::Error>
    where
        D: embedded_graphics::prelude::DrawTarget<Color = C>,
        C: embedded_graphics::prelude::PixelColor
            + From<embedded_graphics::pixelcolor::BinaryColor>,
    {
        match self {
            Self::Dashboard => draw_display(display, data),
            Self::Foiling => draw_foiling(display, data),
        }
    }
}

impl core::fmt::Display for Layout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// The name was not one of [`Layout::ALL`].
///
/// Implements [`core::error::Error`] so clap's `FromStr` value parser accepts
/// [`Layout`] directly, without the front-ends each wrapping it in a local enum.
#[derive(Debug)]
pub struct UnknownLayout;

impl core::error::Error for UnknownLayout {}

impl core::fmt::Display for UnknownLayout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "expected one of")?;
        for (index, layout) in Layout::ALL.iter().enumerate() {
            write!(f, "{} {layout}", if index == 0 { "" } else { "," })?;
        }
        Ok(())
    }
}

impl core::str::FromStr for Layout {
    type Err = UnknownLayout;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|layout| layout.name() == s)
            .ok_or(UnknownLayout)
    }
}

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
    pub gnss_fix: DisplayValue<GnssFix>,
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
    /// The VESC's own motor NTC input, from status message 4. Recorded because the
    /// VESC still sends it, but **not shown**: the reading is broken on this boat,
    /// which is why the standalone node below exists.
    pub vesc_motor_temperature: DisplayValue<f32>,
    /// The standalone motor NTC node, `0x219` -- the motor temperature the display
    /// shows.
    ///
    /// Nested `Option` on purpose. The outer one is the node's liveness, the inner
    /// one is whether the reading it sent was usable, and the display needs both:
    /// silence and a reported fault are different facts even though they draw the
    /// same dashes.
    pub motor_ntc_temperature: DisplayValue<Option<f32>>,
    pub throttle_value: DisplayValue<f32>,
    pub throttle_errors: DisplayValue<ThrottleErrors>,
    pub mppt_panel_info: [DisplayValue<(f32, f32, f32)>; MPPT_PANEL_COUNT], // (Power, Voltage, Current), indexed by boat-position - 1
    /// Hottest temperature each MPPT reports, in °C, indexed by ID strap -- not by
    /// boat position, so a unit reports regardless of whether `LAYOUT` places it.
    /// GaN units report a board and a heat sink temperature; this keeps the higher
    /// of the two. Legacy MPPTs have no strap and are not covered.
    pub mppt_temperatures: [DisplayValue<i8>; GAN_STRAP_COUNT],
    pub charging_disabled: DisplayValue<bool>,
    pub time: DisplayValue<GnssDateTime>,
    pub ip_address: DisplayValue<Ipv4Addr>,
    pub display_state_of_charge: DisplayValue<f32>,
    pub display_is_charging: DisplayValue<bool>,
    pub height_sensor_front_left: DisplayValue<u16>,
    pub height_sensor_front_right: DisplayValue<u16>,
    pub temperature_height_sensors_controller: DisplayValue<f32>,
    pub temperature_rudder_controller: DisplayValue<f32>,
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
                    self.vesc_motor_temperature.update(motor_temp);
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
                let node = mppt_data.node_id();
                let channel_power = match mppt_data.inner() {
                    MpptInfo::Channel0(MpptChannel::Power(p)) => Some((0u8, p)),
                    MpptInfo::Channel1(MpptChannel::Power(p)) => Some((1u8, p)),
                    MpptInfo::Channel2(MpptChannel::Power(p)) => Some((2u8, p)),
                    MpptInfo::Channel3(MpptChannel::Power(p)) => Some((3u8, p)),
                    // Legacy MPPTs report a node temperature, but they have no ID
                    // strap to name them by, so it is not surfaced.
                    _ => None,
                };
                if let Some((channel, power)) = channel_power {
                    if let Some(pos) = position_of(MpptKind::Legacy { node, channel }) {
                        self.mppt_panel_info[pos as usize - 1].update((
                            power.voltage_in * power.current_in,
                            power.voltage_in,
                            power.current_in,
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
                    self.gnss_fix.update(GnssFix::from_code(data.fix));
                }
                GnssData::GnssLatitude(_) => {}
                GnssData::GnssLongitude(_) => {}
            },
            EoiCanData::RudderController(_) => {}
            EoiCanData::HeightSensors(height) => match height {
                HeightSensorData::FrontLeft(status) => {
                    self.height_sensor_front_left.update(status.value);
                }
                HeightSensorData::FrontRight(status) => {
                    self.height_sensor_front_right.update(status.value);
                }
                _ => {}
            },
            EoiCanData::GanMppt(gan_data) => {
                let node = gan_data.node_id();
                match gan_data.inner() {
                    GanMpptPacket::Power(power) => {
                        if let Some(pos) = position_of(MpptKind::Gan { node }) {
                            self.mppt_panel_info[pos as usize - 1].update((
                                power.input_voltage * power.input_current,
                                power.input_voltage,
                                power.input_current,
                            ));
                        }
                    }
                    GanMpptPacket::Status(status) => {
                        // Indexed by strap: every MPPT on the bus reports, whether
                        // or not LAYOUT gives it a boat position. The heat sink
                        // usually leads the board, so take whichever is hotter.
                        if let Some(slot) = self.mppt_temperatures.get_mut(node as usize) {
                            slot.update(status.board_temp.max(status.heat_sink_temp));
                        }
                    }
                    _ => {}
                }
            }
            EoiCanData::Temperature(temp) => match temp {
                TemperatureData::HeightSensorsController(value) => {
                    self.temperature_height_sensors_controller.update(value);
                }
                TemperatureData::RudderController(value) => {
                    self.temperature_rudder_controller.update(value);
                }
                TemperatureData::MotorNtc(ntc) => {
                    // Store the frame's verdict, fault included, so hearing from the
                    // node counts as fresh data even when it has no reading to give.
                    // The status flags say why, which the display has no room for.
                    self.motor_ntc_temperature.update(ntc.temperature);
                }
            },
            EoiCanData::DataLogger(DataLoggerData::WifiIp(octets)) => {
                self.ip_address.update(Ipv4Addr::from(octets));
            }
        }
    }

    pub fn update_cell_voltages(&mut self, offset: usize, values: &[f32]) {
        for (index, value) in values.iter().enumerate() {
            self.battery_cell_voltages[offset + index].update(*value);
        }
    }

    /// The hottest MPPT currently reporting, and how to name it. `None` while no
    /// MPPT has sent a temperature.
    pub fn hottest_mppt(&self) -> Option<(MpptId, i8)> {
        self.mppt_temperatures
            .iter()
            .enumerate()
            .filter_map(|(strap, value)| value.get().map(|t| (MpptId::of_strap(strap as u8), *t)))
            .max_by_key(|(_, t)| *t)
    }

    /// Motor temperature in °C, from the standalone `0x219` node.
    ///
    /// The VESC's own reading is deliberately not a fallback. It is broken, so
    /// falling back to it would replace honest dashes with a wrong number -- and a
    /// wrong motor temperature is worse than none, since this is what the
    /// over-temperature icon is decided on.
    pub fn motor_temperature(&self) -> Option<f32> {
        self.motor_ntc_temperature.get().copied().flatten()
    }

    /// The hottest of the battery's four pack thermistors, in °C.
    pub fn hottest_battery_temperature(&self) -> Option<i8> {
        self.battery_temperatures
            .iter()
            .filter_map(|value| value.get().copied())
            .max()
    }
}

/// GNSS fix quality, from byte 0 of `0x200`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GnssFix {
    None,
    /// Latitude and longitude only.
    Fix2D,
    Fix3D,
}

impl GnssFix {
    /// Codes are as `CAN_MESSAGES.md` documents them: 1 is 3D and 2 is 2D, which
    /// looks backwards but keeps 1 meaning what it always did. Anything else is
    /// treated as no fix rather than guessed at.
    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Fix3D,
            2 => Self::Fix2D,
            _ => Self::None,
        }
    }
}

/// How an MPPT is named on screen: the side and 0-based position its ID strap
/// encodes -- `F0`-`F7` forward, `R0`-`R7` aft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpptId {
    pub side: Side,
    pub position: u8,
}

impl MpptId {
    fn of_strap(strap: u8) -> Self {
        let (side, position) = gan_side_and_position(strap);
        Self { side, position }
    }
}
