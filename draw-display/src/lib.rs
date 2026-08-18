#![cfg_attr(not(test), no_std)]

mod render;
mod time;

pub use render::dashboard::draw_display;
pub use render::foiling::draw_foiling;
pub use render::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

use core::net::Ipv4Addr;

use eoi_can_decoder::{
    BatteryState, ChargeState, DataLoggerData, DischargeState, EoiBattery, EoiCanData,
    FoilTuneValue, GanMpptPacket, GnssData, GnssDateTime, HeightSensorData, MpptChannel, MpptInfo,
    TemperatureData, ThrottleData, ThrottleErrors, VescData,
};
use mppt_layout::{gan_side_and_position, position_of, MpptKind, Side, GAN_STRAP_COUNT, LAYOUT};
use render::foiling::{cell_for_index, PairHalf};

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

/// A parameter value: one number, or an up/down pair that has diverged.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum Reading {
    /// One number, or an up/down pair that currently agrees.
    One(f32),
    /// An up/down pair that has diverged, drawn as `up^ down v`.
    UpDown(f32, f32),
}

/// Which value column a cell is in. The row is the screen row, so the
/// datalogger addresses a cell without either side agreeing a parameter numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum FoilColumn {
    Pitch,
    Roll,
    /// The height and rear loops.
    Mid,
    /// Turn, mode and the global scaling speed.
    Right,
    /// A config slot.
    Slot,
}

/// The cell the datalogger has selected, drawn inverted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct FoilCursor {
    pub column: FoilColumn,
    /// Screen row, the header being row 0.
    pub row: u8,
}

/// The last parameter change, for the status line.
///
/// `from` is the value before the current edit *burst*, not before the last
/// keypress: holding `+` walks `to` upwards while `from` stays put, so the line
/// reads as one movement rather than a stream of single steps. The datalogger
/// owns that distinction -- it decides when a burst starts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct FoilEdit {
    pub column: FoilColumn,
    pub row: u8,
    pub from: f32,
    pub to: f32,
}

/// Everything the foiling screen draws.
///
/// Deliberately not one flat array indexed by ArduPilot parameter number: the
/// screen is addressed by column and row, so a renumbering upstream cannot
/// silently move a value to the wrong row.
#[derive(Debug, Default)]
pub struct FoilingData {
    /// The axis rate loops, one entry per screen row.
    pub pitch: [DisplayValue<Reading>; 13],
    pub roll: [DisplayValue<Reading>; 13],
    pub height: [DisplayValue<Reading>; 7],
    pub rear: [DisplayValue<Reading>; 4],
    pub turn: [DisplayValue<Reading>; 6],
    pub mode: [DisplayValue<Reading>; 4],
    pub global: [DisplayValue<Reading>; 1],
    /// Store time per slot: `None` never stored, `Some(None)` stored without a
    /// GNSS fix, `Some(Some((h, m)))` stored at that time.
    pub slots: [DisplayValue<Option<(u8, u8)>>; 9],
    pub cursor: DisplayValue<FoilCursor>,
    /// Held until the next edit arrives rather than timing out: the point of the
    /// line is to still be readable a while after the change.
    pub last_edit: Option<FoilEdit>,
    /// Both halves of each collapsed up/down pair, kept raw.
    ///
    /// A pair arrives as two separate `0x261` frames, so writing straight into
    /// the cell would lose whichever half came first. Indexed by
    /// [`FoilingData::PAIRS`]: pitch RMAX, pitch LIMIT, height CMD.
    pairs: [(Option<f32>, Option<f32>); 3],
}

impl FoilingData {
    /// The cells that hold an up/down pair, in `pairs` order.
    const PAIRS: [(FoilColumn, u8); 3] = [
        (FoilColumn::Pitch, 7),
        (FoilColumn::Pitch, 8),
        (FoilColumn::Mid, 6),
    ];

    fn values_mut(&mut self, column: FoilColumn, row: u8) -> Option<&mut DisplayValue<Reading>> {
        let index = usize::from(row);
        match column {
            FoilColumn::Pitch => self.pitch.get_mut(index - 1),
            FoilColumn::Roll => self.roll.get_mut(index - 1),
            // The stacked columns are several tables deep, so the row has to be
            // resolved against the same block layout the renderer draws from.
            FoilColumn::Mid | FoilColumn::Right => {
                type Blocks<'a> = (&'a [(u8, u8)], [&'a mut [DisplayValue<Reading>]; 3]);
                let (offsets, arrays): Blocks<'_> = if matches!(column, FoilColumn::Mid) {
                    (
                        &[(1, 7), (9, 4)],
                        [&mut self.height, &mut self.rear, &mut []],
                    )
                } else {
                    (
                        &[(1, 6), (8, 4), (12, 1)],
                        [&mut self.turn, &mut self.mode, &mut self.global],
                    )
                };
                for (&(first, len), array) in offsets.iter().zip(arrays) {
                    if row >= first && row < first + len {
                        return array.get_mut(usize::from(row - first));
                    }
                }
                None
            }
            FoilColumn::Slot => None,
        }
    }

    /// Apply one `0x261` read-back.
    fn ingest_param(&mut self, index: u8, value: Option<f32>) {
        let Some((column, row, half)) = cell_for_index(index) else {
            return;
        };

        let reading = match half {
            PairHalf::Whole => value.map(Reading::One),
            _ => {
                let slot = Self::PAIRS
                    .iter()
                    .position(|&cell| cell == (column, row))
                    .expect("every paired index maps to a PAIRS cell");
                let pair = &mut self.pairs[slot];
                if matches!(half, PairHalf::Up) {
                    pair.0 = value;
                } else {
                    pair.1 = value;
                }
                // Published only once both halves are known: half a pair cannot be
                // drawn honestly, and the tuner dumps the whole table at connect,
                // so the gap is brief.
                match *pair {
                    (Some(up), Some(down)) => Some(Reading::UpDown(up, down)),
                    _ => None,
                }
            }
        };

        let Some(reading) = reading else {
            return;
        };

        // The status line's `from` is the value before this burst, so a run of
        // steps on one cell reads as a single movement. A different cell starts a
        // new burst.
        if let Some(previous) = self.values_mut(column, row).and_then(|v| v.get().copied()) {
            let (was, is) = (first_of(previous), first_of(reading));
            if (was - is).abs() > f32::EPSILON {
                let from = match self.last_edit {
                    Some(edit) if edit.column == column && edit.row == row => edit.from,
                    _ => was,
                };
                self.last_edit = Some(FoilEdit {
                    column,
                    row,
                    from,
                    to: is,
                });
            }
        }

        if let Some(slot) = self.values_mut(column, row) {
            slot.update(reading);
        }

        // The tuner re-requests the selected cell once the cursor settles, so the
        // most recent read-back is where it is. Fragile by construction -- during
        // the connect-time dump this walks the whole table before settling -- but
        // it is the mechanism the protocol offers, and the re-request corrects it
        // within a second. An explicit cursor frame would be better.
        self.cursor.update(FoilCursor { column, row });
    }
}

/// The number a reading leads with, for comparing one against another.
fn first_of(reading: Reading) -> f32 {
    match reading {
        Reading::One(v) | Reading::UpDown(v, _) => v,
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
    /// Only the foiling layout draws these; the dashboard ignores them.
    pub foiling: FoilingData,
}

impl DisplayData {
    pub fn ingest_eoi_can_data(&mut self, data: EoiCanData) {
        match data {
            EoiCanData::FoilTune(value) => match value {
                FoilTuneValue::Param {
                    index,
                    status,
                    value,
                } => {
                    // A status other than ok/clamped/locked is sent with a value of
                    // zero, so the float is dropped rather than drawn: an
                    // unavailable `HYD_*` must read as dashes, not as a gain of 0.
                    self.foiling
                        .ingest_param(index, status.has_value().then_some(value));
                }
                // Neither is a parameter, and the display needs neither: the
                // version is the tuner's concern and the dump marker only brackets
                // a burst.
                FoilTuneValue::ProtocolVersion(_) | FoilTuneValue::DumpComplete(_) => {}
            },
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

#[cfg(test)]
mod foil_ingest_tests {
    use super::*;
    use embedded_can::{Id, StandardId};
    use eoi_can_decoder::can_frame::CanFrame;
    use eoi_can_decoder::parse_eoi_can_data;

    /// One `0x261 PARAM_VALUE` frame, as `foil_tune.lua` sends it.
    fn value_frame(index: u8, status: u8, value: f32) -> CanFrame {
        let bytes = value.to_le_bytes();
        CanFrame::from_encoded(
            Id::Standard(StandardId::new(0x261).unwrap()),
            &[index, status, bytes[0], bytes[1], bytes[2], bytes[3]],
        )
    }

    fn feed(data: &mut DisplayData, index: u8, status: u8, value: f32) {
        let parsed = parse_eoi_can_data(&value_frame(index, status, value)).expect("decodes");
        data.ingest_eoi_can_data(parsed);
    }

    /// Index 16 is `PTCH_RATE_P`, which the screen draws at Pitch row 1.
    #[test]
    fn a_readback_lands_in_its_cell() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 2.93);
        assert_eq!(
            data.foiling.pitch[0].get().copied(),
            Some(Reading::One(2.93))
        );
        // The roll side of the same row is a different parameter and untouched.
        assert_eq!(data.foiling.roll[0].get().copied(), None);
        feed(&mut data, 1, 0, 0.33);
        assert_eq!(
            data.foiling.roll[0].get().copied(),
            Some(Reading::One(0.33))
        );
    }

    /// A parameter that does not exist yet is sent with a value of zero, which must
    /// not be drawn: `HYD_*` reads back unavailable until `hydrofoils.lua` has
    /// created them, and a gain of 0 would look like a deliberate setting.
    #[test]
    fn an_unavailable_parameter_leaves_the_cell_empty() {
        let mut data = DisplayData::default();
        feed(&mut data, 32, 4, 0.0);
        assert_eq!(data.foiling.height[0].get().copied(), None);
        // Locked still carries the live value, so that one lands.
        feed(&mut data, 32, 5, 1200.0);
        assert_eq!(
            data.foiling.height[0].get().copied(),
            Some(Reading::One(1200.0))
        );
    }

    /// The height command clamps are two parameters in one cell, so neither half
    /// may overwrite the other, and nothing is drawn until both have arrived.
    #[test]
    fn a_pair_needs_both_halves() {
        let mut data = DisplayData::default();
        feed(&mut data, 52, 0, 5.0); // HYD_CMDMAX
        assert_eq!(
            data.foiling.height[5].get().copied(),
            None,
            "half a pair cannot be drawn honestly"
        );
        feed(&mut data, 53, 0, -8.0); // HYD_CMDMIN
        assert_eq!(
            data.foiling.height[5].get().copied(),
            Some(Reading::UpDown(5.0, -8.0))
        );
        // A later update to one half keeps the other.
        feed(&mut data, 52, 0, 4.5);
        assert_eq!(
            data.foiling.height[5].get().copied(),
            Some(Reading::UpDown(4.5, -8.0))
        );
    }

    /// The pitch rate max is a pair too, and reads as one number while its halves
    /// agree -- the renderer collapses it, so both are always stored.
    #[test]
    fn a_symmetric_pair_is_still_stored_as_two_halves() {
        let mut data = DisplayData::default();
        feed(&mut data, 22, 0, 60.0);
        feed(&mut data, 23, 0, 60.0);
        assert_eq!(
            data.foiling.pitch[6].get().copied(),
            Some(Reading::UpDown(60.0, 60.0))
        );
    }

    /// A run of steps on one cell reads as a single movement: `from` is the value
    /// the burst started at, not the previous keypress.
    #[test]
    fn an_edit_burst_holds_its_starting_value() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 2.10);
        assert!(
            data.foiling.last_edit.is_none(),
            "the first read is not an edit"
        );

        feed(&mut data, 16, 0, 2.15);
        feed(&mut data, 16, 0, 2.20);
        let edit = data.foiling.last_edit.expect("an edit");
        assert_eq!((edit.column, edit.row), (FoilColumn::Pitch, 1));
        assert!(
            (edit.from - 2.10).abs() < 1e-6,
            "from held across the burst"
        );
        assert!((edit.to - 2.20).abs() < 1e-6);

        // A different cell starts a new burst.
        feed(&mut data, 1, 0, 0.33);
        feed(&mut data, 1, 0, 0.35);
        let edit = data.foiling.last_edit.expect("an edit");
        assert_eq!((edit.column, edit.row), (FoilColumn::Roll, 1));
        assert!((edit.from - 0.33).abs() < 1e-6);
    }

    /// The tuner re-requests the selected cell once the cursor settles, so the most
    /// recent read-back is where it is.
    #[test]
    fn the_cursor_follows_the_last_readback() {
        let mut data = DisplayData::default();
        feed(&mut data, 44, 0, 4.0); // TRN_RATE -> Right row 5
        assert_eq!(
            data.foiling.cursor.get().copied(),
            Some(FoilCursor {
                column: FoilColumn::Right,
                row: 5
            })
        );
    }

    /// The cursor is inferred from the last read-back, which is what the protocol
    /// offers -- there is no cursor frame. During the connect-time dump that walks
    /// the whole table and ends up wherever the dump ended, which is wrong; the
    /// tuner's re-request of the selected cell then corrects it.
    ///
    /// This is the weakest part of the wire contract and the test says so: an
    /// explicit cursor frame would remove the transient entirely.
    #[test]
    fn the_cursor_settles_after_a_dump_corrects_it() {
        let mut data = DisplayData::default();
        // A dump, in index order, ending on SCR_USER4 -> Right row 11.
        for index in [16, 17, 32, 40, 48, 49, 50, 51] {
            feed(&mut data, index, 0, 1.0);
        }
        assert_eq!(
            data.foiling.cursor.get().copied(),
            Some(FoilCursor {
                column: FoilColumn::Right,
                row: 11
            }),
            "mid-dump the cursor sits wherever the dump ended"
        );

        // The tuner re-requests the selected cell once the cursor settles.
        feed(&mut data, 16, 0, 1.0);
        assert_eq!(
            data.foiling.cursor.get().copied(),
            Some(FoilCursor {
                column: FoilColumn::Pitch,
                row: 1
            }),
            "the re-request puts it where the operator actually is"
        );
    }

    /// Every index in the tuner's table has to land somewhere drawable, or a
    /// parameter would arrive and vanish.
    #[test]
    fn every_tuner_index_maps_to_a_cell() {
        let indices = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
            28, 29, 30, 31, 32, 33, 34, 35, 36, 39, 40, 41, 42, 43, 44, 45, 48, 49, 50, 51, 52, 53,
            54, 55, 56, 57,
        ];
        assert_eq!(indices.len(), 50, "foil_tune.lua PROTO_VERSION 7 has 50");
        let mut data = DisplayData::default();
        for index in indices {
            let cell = render::foiling::cell_for_index(index);
            assert!(cell.is_some(), "index {index} has no cell");
            let (column, row, _) = cell.unwrap();
            // Reachable in the data as well as on the map.
            feed(&mut data, index, 0, 1.0);
            assert!(
                data.foiling.values_mut(column, row).is_some(),
                "index {index} maps to {column:?} row {row}, which has no slot"
            );
        }
        // Retired and unused indices must not resolve.
        for index in [0, 13, 14, 15, 37, 38, 46, 47, 58, 200, 0xFD] {
            assert!(
                render::foiling::cell_for_index(index).is_none(),
                "index {index} should not map anywhere"
            );
        }
    }
}
