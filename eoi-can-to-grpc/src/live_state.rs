//! Stateful aggregation on top of `eoi-can-decoder`'s stateless per-frame parser:
//! staleness (5 s per field), derived values (net/in/out power, hottest MPPT/battery
//! thermistor), and the `Warnings`/`BusHealth` roll-ups the Snapshot proto needs. None
//! of this lives in `eoi-can-decoder` itself (it's a pure `parse_eoi_can_data`, no
//! notion of time) or in `eoi-can-to-mqtt`'s `FrameCache` (that caches whole frames at
//! one TTL per CAN ID; this needs per-field freshness, e.g. the four separate BMS
//! cell-group messages and the four separate MPPT channel messages).
//!
//! Ported from eoi-grpc-telemetry's `eoi-can-decode` crate, which this replaces —
//! see that crate's `state.rs` for the version this was adapted from. Behavioural
//! differences from that port, all deliberate:
//!
//! - **`discharge_a`'s sign flipped.** `eoi-can-decoder` negates the raw wire value
//!   for `0x101` discharge current (`ChargeAndDischargeCurrent`), matching
//!   CAN_MESSAGES.md's documented "negated on wire" convention and its own regression
//!   test; the old `eoi-can-decode` did not negate it. So `charge_a` positive = charging
//!   and `discharge_a` **negative** = discharging, symmetric with each other — whereas
//!   the old crate exposed `discharge_a` as a positive magnitude. Any dashboard reading
//!   of `Battery.discharge_a` should expect the new sign.
//! - **MPPT per-channel conflation bug fixed.** The old decoder mapped all four
//!   channel vin/iin sub-messages (field IDs 0/2/4/6) of one MPPT node into a single
//!   slot, so each new channel frame silently overwrote the last — `MpptView` only
//!   ever reflected whichever channel most recently transmitted. This version tracks
//!   the four channels independently and reduces them to one vin/iin pair per node as
//!   a current-weighted average voltage plus summed current (see `mppt_view` below) —
//!   an explicit design choice, not a rediscovery of the old (accidental) behaviour.
//! - **GaN heat-sink temperature now available.** The old decoder's GaN status parse
//!   only captured `board_temp` (and mislabeled it via a byte offset that actually reads
//!   `board_temp`, not `heat_sink_temp`). `eoi-can-decoder` exposes both; `hottest_mppt`
//!   now considers whichever of the two is hotter for a GaN node.
//! - **Retired `0x217` and VESC status-4's `motor_temp` are never read into any live
//!   field.** `eoi-can-decoder` still decodes both (so archived candump logs replay
//!   cleanly), but this layer deliberately never routes either into `motor_celsius` —
//!   only `TemperatureData::MotorNtc` (`0x219`) may set it. Getting this wrong is the
//!   one mistake that would make the web dashboard silently disagree with the pilot's
//!   own display, which already only trusts `0x219`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use eoi_can_decoder::can_frame::CanFrame;
use eoi_can_decoder::{
    parse_eoi_can_data, BatteryState, ChargeState, DischargeState, EoiBattery, EoiCanData,
    GanMpptPacket, GanPhaseFault, GanPhaseMode, GnssData, HeightSensorData, HeightSensorState,
    MpptChannel, MpptInfo, RudderControllerData, ServoData, ServoFaultCause, ServoState,
    TemperatureData, ThrottleData, ThrottleErrors, ThrottleTwiErrors, VescData,
};
use embedded_can::{ExtendedId, Id, StandardId};

pub const STALE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    Ok,
    Clamped,
    Settling,
    Absent,
}

#[derive(Debug, Clone, Copy)]
struct Timed<T> {
    value: T,
    at: Instant,
}

impl<T: Copy> Timed<T> {
    fn fresh(&self, now: Instant) -> Option<T> {
        if now.saturating_duration_since(self.at) <= STALE_AFTER {
            Some(self.value)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MpptChannelSlot {
    vin: Option<Timed<f32>>,
    iin: Option<Timed<f32>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct MpptSlot {
    /// Channels 0..3 for a conventional MPPT; only index 0 is ever populated for a
    /// GaN node (it has one input stage, not four).
    channels: [MpptChannelSlot; 4],
    vout: Option<Timed<f32>>,
    iout: Option<Timed<f32>>,
    /// MPPT board temperature; GaN node's `board_temp`.
    celsius: Option<Timed<f32>>,
    /// GaN only: `heat_sink_temp`, considered alongside `celsius` for "hottest".
    heat_sink_celsius: Option<Timed<f32>>,
    state: Option<Timed<u32>>,
    flags: u32,
    gan: bool,
}

#[derive(Debug, Default)]
pub struct LiveState {
    pack_a: Option<Timed<f32>>,
    peri_a: Option<Timed<f32>>,
    charge_a: Option<Timed<f32>>,
    discharge_a: Option<Timed<f32>>,
    soc_pct: Option<Timed<f32>>,
    error_flags: u32,
    balancing_flags: u32,
    cells: [Option<Timed<f32>>; 14],
    cell_group_at: [Option<Instant>; 4],
    pack_v: Option<Timed<f32>>,
    stack_v: Option<Timed<f32>>,
    thermistors: [Option<Timed<i8>>; 4],
    ic_temp: Option<Timed<i8>>,
    batt_state: Option<Timed<u32>>,
    charge_state: Option<Timed<u32>>,
    discharge_state: Option<Timed<u32>>,
    uptime_ms: Option<Timed<u32>>,
    gnss_fix: Option<Timed<u32>>,
    gnss_sats: Option<Timed<u32>>,
    gnss_sats_used: Option<Timed<u32>>,
    speed_kmh: Option<Timed<f32>>,
    heading_deg: Option<Timed<f32>>,
    lat: Option<Timed<f64>>,
    lon: Option<Timed<f64>>,
    heights: [Option<Timed<(u32, f32)>>; 4],
    rudder_setpoint: Option<Timed<u32>>,
    rudder_actual: Option<Timed<u32>>,
    rudder_state: Option<Timed<u32>>,
    rudder_fault: Option<Timed<u32>>,
    steering_deg: Option<Timed<f32>>,
    rudder_ctl_c: Option<Timed<f32>>,
    height_ctl_c: Option<Timed<f32>>,
    flow_in: Option<Timed<f32>>,
    flow_out: Option<Timed<f32>>,
    motor_celsius: Option<Timed<f32>>,
    motor_quality: Quality,
    rpm: Option<Timed<f32>>,
    motor_current_a: Option<Timed<f32>>,
    motor_duty: Option<Timed<f32>>,
    tacho: Option<Timed<i32>>,
    fet_celsius: Option<Timed<f32>>,
    ah_used: Option<Timed<f32>>,
    ah_gen: Option<Timed<f32>>,
    wh_used: Option<Timed<f32>>,
    wh_gen: Option<Timed<f32>>,
    input_v: Option<Timed<f32>>,
    throttle_duty: Option<Timed<f32>>,
    throttle_current: Option<Timed<f32>>,
    throttle_rpm: Option<Timed<f32>>,
    throttle_rel: Option<Timed<f32>>,
    throttle_pos: Option<Timed<f32>>,
    throttle_errors: u32,
    throttle_err_at: Option<Instant>,
    mppts: BTreeMap<(bool, u32), MpptSlot>,
    frames_total: u64,
    frames_unknown: u64,
    frame_times: Vec<Instant>,
}

impl LiveState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note_unknown(&mut self, now: Instant) {
        self.frames_total += 1;
        self.frames_unknown += 1;
        self.push_frame_time(now);
    }

    pub fn apply(&mut self, update: EoiCanData, now: Instant) {
        self.frames_total += 1;
        self.push_frame_time(now);
        match update {
            EoiCanData::EoiBattery(b) => self.apply_battery(b, now),
            EoiCanData::Vesc(v) => self.apply_vesc(v, now),
            EoiCanData::Throttle(t) => self.apply_throttle(t, now),
            EoiCanData::Mppt(m) => {
                let id = m.node_id() as u32;
                let slot = self.mppts.entry((false, id)).or_default();
                slot.gan = false;
                apply_mppt_info(slot, m.inner(), now);
            }
            EoiCanData::GanMppt(g) => {
                let id = g.node_id() as u32;
                let slot = self.mppts.entry((true, id)).or_default();
                slot.gan = true;
                apply_gan_packet(slot, g.inner(), now);
            }
            EoiCanData::Gnss(g) => self.apply_gnss(g, now),
            EoiCanData::RudderController(r) => self.apply_rudder(r, now),
            EoiCanData::HeightSensors(h) => self.apply_height(h, now),
            EoiCanData::Temperature(t) => self.apply_temperature(t, now),
            // No live field consumes these; decoded successfully but nothing to do.
            EoiCanData::DataLogger(_) => {}
        }
    }

    fn apply_battery(&mut self, b: EoiBattery, now: Instant) {
        match b {
            EoiBattery::PackAndPerriCurrent(p) => {
                self.pack_a = Some(Timed { value: p.pack_current, at: now });
                self.peri_a = Some(Timed { value: p.perri_current, at: now });
            }
            EoiBattery::ChargeAndDischargeCurrent(c) => {
                self.charge_a = Some(Timed { value: c.charge_current, at: now });
                self.discharge_a = Some(Timed { value: c.discharge_current, at: now });
            }
            EoiBattery::SocErrorFlagsAndBalancing(s) => {
                self.soc_pct = Some(Timed { value: s.state_of_charge, at: now });
                self.error_flags = s.error_flags;
                self.balancing_flags = s.balancing_status as u32;
            }
            EoiBattery::CellVoltages1_4(c) => self.apply_cell_group(0, &c.cell_voltage, now),
            EoiBattery::CellVoltages5_8(c) => self.apply_cell_group(1, &c.cell_voltage, now),
            EoiBattery::CellVoltages9_12(c) => self.apply_cell_group(2, &c.cell_voltage, now),
            EoiBattery::CellVoltages13_14PackAndStack(c) => {
                self.cells[12] = Some(Timed { value: c.cell_voltage[0], at: now });
                self.cells[13] = Some(Timed { value: c.cell_voltage[1], at: now });
                self.cell_group_at[3] = Some(now);
                self.pack_v = Some(Timed { value: c.pack_voltage, at: now });
                self.stack_v = Some(Timed { value: c.stack_voltage, at: now });
            }
            EoiBattery::TemperaturesAndStates(t) => {
                for (i, temp) in t.temperatures.iter().enumerate() {
                    self.thermistors[i] = Some(Timed { value: *temp, at: now });
                }
                self.ic_temp = Some(Timed { value: t.ic_temperature, at: now });
                self.batt_state = Some(Timed { value: battery_state_num(&t.battery_state), at: now });
                self.charge_state = Some(Timed { value: charge_state_num(&t.charge_state), at: now });
                self.discharge_state =
                    Some(Timed { value: discharge_state_num(&t.discharge_state), at: now });
            }
            EoiBattery::BatteryUptime(u) => {
                self.uptime_ms = Some(Timed { value: u.uptime_ms, at: now });
            }
        }
    }

    fn apply_cell_group(&mut self, group: usize, volts: &[f32; 4], now: Instant) {
        let start = group * 4;
        for (i, v) in volts.iter().enumerate() {
            self.cells[start + i] = Some(Timed { value: *v, at: now });
        }
        self.cell_group_at[group] = Some(now);
    }

    fn apply_vesc(&mut self, v: VescData, now: Instant) {
        match v {
            VescData::StatusMessage1 { rpm, total_current, duty_cycle } => {
                self.rpm = Some(Timed { value: rpm as f32, at: now });
                self.motor_current_a = Some(Timed { value: total_current, at: now });
                self.motor_duty = Some(Timed { value: duty_cycle, at: now });
            }
            VescData::StatusMessage2 { amp_hours_used, amp_hours_generated } => {
                self.ah_used = Some(Timed { value: amp_hours_used, at: now });
                self.ah_gen = Some(Timed { value: amp_hours_generated, at: now });
            }
            VescData::StatusMessage3 { watt_hours_used, watt_hours_generated } => {
                self.wh_used = Some(Timed { value: watt_hours_used, at: now });
                self.wh_gen = Some(Timed { value: watt_hours_generated, at: now });
            }
            VescData::StatusMessage4 { fet_temp, motor_temp: _, total_input_current: _, current_pid_position: _ } => {
                // motor_temp is deliberately never read — broken on this boat; motor
                // temperature comes only from TemperatureData::MotorNtc (0x219).
                self.fet_celsius = Some(Timed { value: fet_temp, at: now });
            }
            VescData::StatusMessage5 { input_voltage, tachometer } => {
                self.tacho = Some(Timed { value: tachometer, at: now });
                self.input_v = Some(Timed { value: input_voltage, at: now });
            }
        }
    }

    fn apply_throttle(&mut self, t: ThrottleData, now: Instant) {
        match t {
            ThrottleData::ToVescDutyCycle(v) => {
                self.throttle_duty = Some(Timed { value: v, at: now });
            }
            ThrottleData::ToVescCurrent(v) => {
                self.throttle_current = Some(Timed { value: v, at: now });
            }
            ThrottleData::ToVescRpm(v) => {
                self.throttle_rpm = Some(Timed { value: v, at: now });
            }
            ThrottleData::ToVescCurrentRelative(v) => {
                self.throttle_rel = Some(Timed { value: v, at: now });
            }
            ThrottleData::Status(s) => {
                self.throttle_pos = Some(Timed { value: s.value, at: now });
                self.throttle_errors = throttle_error_bits(&s.error);
                self.throttle_err_at = Some(now);
            }
            // Config frames carry no telemetry; nothing to update.
            ThrottleData::Config(_) => {}
        }
    }

    fn apply_gnss(&mut self, g: GnssData, now: Instant) {
        match g {
            GnssData::GnssStatus(s) => {
                self.gnss_fix = Some(Timed { value: s.fix as u32, at: now });
                self.gnss_sats = Some(Timed { value: s.sats as u32, at: now });
                self.gnss_sats_used = Some(Timed { value: s.sats_used as u32, at: now });
            }
            GnssData::GnssSpeedAndHeading(speed_kmh, heading_deg) => {
                self.speed_kmh = Some(Timed { value: speed_kmh, at: now });
                self.heading_deg = Some(Timed { value: heading_deg, at: now });
            }
            GnssData::GnssLatitude(lat) => self.lat = Some(Timed { value: lat, at: now }),
            GnssData::GnssLongitude(lon) => self.lon = Some(Timed { value: lon, at: now }),
            // Not part of the Snapshot; the bridge doesn't need the boat's own clock.
            GnssData::GnssDateTime(_) => {}
        }
    }

    fn apply_rudder(&mut self, r: RudderControllerData, now: Instant) {
        match r {
            RudderControllerData::Servo(ServoData::Setpoint(sp)) => {
                self.rudder_setpoint = Some(Timed { value: sp as u32, at: now });
            }
            RudderControllerData::Servo(ServoData::Status(s)) => {
                self.rudder_state = Some(Timed { value: servo_state_num(&s.state), at: now });
                self.rudder_setpoint = Some(Timed { value: s.setpoint as u32, at: now });
                self.rudder_actual = Some(Timed { value: s.actual_position as u32, at: now });
                self.rudder_fault =
                    Some(Timed { value: servo_fault_num(&s.fault_cause), at: now });
            }
            // Not decoded by the old bridge either; no Snapshot field for it yet.
            RudderControllerData::Servo(ServoData::Command(_)) => {}
            RudderControllerData::CoolingPumpStatus(_) => {}
            RudderControllerData::SteeringAngle(s) => {
                self.steering_deg = Some(Timed { value: s.angle as f32, at: now });
            }
            RudderControllerData::FlowSensorIn(f) => {
                self.flow_in = Some(Timed { value: f.flow_rate as f32, at: now });
            }
            RudderControllerData::FlowSensorOut(f) => {
                self.flow_out = Some(Timed { value: f.flow_rate as f32, at: now });
            }
            // Retired (0x217): kept decodable upstream only so archived logs replay.
            // Never routed into a live field — see the module doc comment.
            RudderControllerData::MotorTemperature(_) => {}
        }
    }

    fn apply_height(&mut self, h: HeightSensorData, now: Instant) {
        let (index, status) = match h {
            HeightSensorData::FrontLeft(s) => (0, s),
            HeightSensorData::FrontRight(s) => (1, s),
            HeightSensorData::Reserved1(s) => (2, s),
            HeightSensorData::Reserved2(s) => (3, s),
        };
        self.heights[index] = Some(Timed {
            value: (height_state_num(&status.state), status.value as f32),
            at: now,
        });
    }

    fn apply_temperature(&mut self, t: TemperatureData, now: Instant) {
        match t {
            TemperatureData::HeightSensorsController(c) => {
                self.height_ctl_c = Some(Timed { value: c, at: now });
            }
            TemperatureData::RudderController(c) => {
                self.rudder_ctl_c = Some(Timed { value: c, at: now });
            }
            TemperatureData::MotorNtc(m) => {
                self.motor_quality = motor_ntc_quality(&m.status, m.temperature.is_some());
                self.motor_celsius = m.temperature.map(|c| Timed { value: c, at: now });
            }
        }
    }

    fn push_frame_time(&mut self, now: Instant) {
        self.frame_times.push(now);
        let cutoff = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
        while self.frame_times.first().is_some_and(|t| *t < cutoff) {
            self.frame_times.remove(0);
        }
    }

    pub fn view(&self, now: Instant) -> SnapshotView {
        let pack_a = self.pack_a.and_then(|t| t.fresh(now));
        let peri_a = self.peri_a.and_then(|t| t.fresh(now));
        let charge_a = self.charge_a.and_then(|t| t.fresh(now));
        let discharge_a = self.discharge_a.and_then(|t| t.fresh(now));
        let pack_v = self.pack_v.and_then(|t| t.fresh(now));
        let motor_a = self.motor_current_a.and_then(|t| t.fresh(now));

        // Wire: charge current is positive-when-charging; discharge current is
        // negative-when-discharging (eoi-can-decoder's convention, see module doc).
        // Positive net power still means charging.
        let net_w = pack_a.zip(pack_v).map(|(i, v)| i * v);
        let in_w = charge_a.zip(pack_v).map(|(i, v)| i * v);
        let out_w = match (motor_a, peri_a, pack_v) {
            (Some(m), Some(p), Some(v)) => Some((m + p.abs()) * v),
            (Some(m), None, Some(v)) => Some(m * v),
            (None, Some(p), Some(v)) => Some(p.abs() * v),
            _ => discharge_a.zip(pack_v).map(|(i, v)| i.abs() * v),
        };

        let cells_all_fresh = self
            .cell_group_at
            .iter()
            .all(|g| g.is_some_and(|at| now.saturating_duration_since(at) <= STALE_AFTER));
        let cell_v = if cells_all_fresh {
            self.cells.iter().filter_map(|c| c.and_then(|t| t.fresh(now))).collect()
        } else {
            Vec::new()
        };

        let batt_temps: Vec<(String, Option<f32>, Quality)> = (0..4)
            .map(|i| {
                let c = self.thermistors[i].and_then(|t| t.fresh(now)).map(|v| v as f32);
                (format!("Cell block {}", i + 1), c, Quality::Ok)
            })
            .chain(std::iter::once({
                let c = self.ic_temp.and_then(|t| t.fresh(now)).map(|v| v as f32);
                ("BMS IC".to_string(), c, Quality::Ok)
            }))
            .collect();

        let mut mppts = Vec::new();
        for ((gan, id), slot) in &self.mppts {
            mppts.push(mppt_view(*gan, *id, slot, now));
        }

        let hottest_mppt = mppts
            .iter()
            .filter_map(|m| m.celsius.map(|c| (m.label.clone(), c)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let hottest_battery = (0..4)
            .filter_map(|i| {
                self.thermistors[i]
                    .and_then(|t| t.fresh(now))
                    .map(|v| (format!("Cell block {}", i + 1), v as f32))
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let motor_c = self.motor_celsius.and_then(|t| t.fresh(now));
        let fet_c = self.fet_celsius.and_then(|t| t.fresh(now));
        let soc = self.soc_pct.and_then(|t| t.fresh(now));
        let batt_state = self.batt_state.and_then(|t| t.fresh(now));
        let chg_state = self.charge_state.and_then(|t| t.fresh(now));
        let dis_state = self.discharge_state.and_then(|t| t.fresh(now));
        let throttle_err_fresh = self
            .throttle_err_at
            .is_some_and(|at| now.saturating_duration_since(at) <= STALE_AFTER);

        let height_labels = ["Front left", "Front right", "Height 3", "Height 4"];
        let heights = (0..4)
            .map(|i| HeightView {
                label: height_labels[i].to_string(),
                state: self.heights[i].and_then(|t| t.fresh(now)).map(|v| v.0),
                height: self.heights[i].and_then(|t| t.fresh(now)).map(|v| v.1),
            })
            .collect();

        SnapshotView {
            boat_uptime_ms: self.uptime_ms.and_then(|t| t.fresh(now)).map(|v| v as u64),
            net_w,
            in_w,
            out_w,
            pack_v,
            pack_a,
            peri_a,
            charge_a,
            discharge_a,
            soc_pct: soc,
            stack_v: self.stack_v.and_then(|t| t.fresh(now)),
            cell_v,
            batt_temps,
            batt_state,
            charge_state: chg_state,
            discharge_state: dis_state,
            error_flags: self.error_flags,
            balancing_flags: self.balancing_flags,
            lat: self.lat.and_then(|t| t.fresh(now)),
            lon: self.lon.and_then(|t| t.fresh(now)),
            speed_kmh: self.speed_kmh.and_then(|t| t.fresh(now)),
            heading_deg: self.heading_deg.and_then(|t| t.fresh(now)),
            gnss_fix: self.gnss_fix.and_then(|t| t.fresh(now)),
            gnss_sats: self.gnss_sats.and_then(|t| t.fresh(now)),
            gnss_sats_used: self.gnss_sats_used.and_then(|t| t.fresh(now)),
            rpm: self.rpm.and_then(|t| t.fresh(now)),
            motor_current_a: motor_a,
            motor_duty: self.motor_duty.and_then(|t| t.fresh(now)),
            tacho: self.tacho.and_then(|t| t.fresh(now)),
            fet_celsius: fet_c,
            motor_celsius: motor_c,
            motor_temp_quality: if motor_c.is_some() { self.motor_quality } else { Quality::Absent },
            ah_used: self.ah_used.and_then(|t| t.fresh(now)),
            ah_gen: self.ah_gen.and_then(|t| t.fresh(now)),
            wh_used: self.wh_used.and_then(|t| t.fresh(now)),
            wh_gen: self.wh_gen.and_then(|t| t.fresh(now)),
            input_v: self.input_v.and_then(|t| t.fresh(now)),
            rudder_setpoint: self.rudder_setpoint.and_then(|t| t.fresh(now)),
            rudder_actual: self.rudder_actual.and_then(|t| t.fresh(now)),
            steering_deg: self.steering_deg.and_then(|t| t.fresh(now)),
            rudder_state: self.rudder_state.and_then(|t| t.fresh(now)),
            rudder_fault: self.rudder_fault.and_then(|t| t.fresh(now)),
            rudder_ctl_c: self.rudder_ctl_c.and_then(|t| t.fresh(now)),
            height_ctl_c: self.height_ctl_c.and_then(|t| t.fresh(now)),
            flow_in: self.flow_in.and_then(|t| t.fresh(now)),
            flow_out: self.flow_out.and_then(|t| t.fresh(now)),
            throttle_duty: self.throttle_duty.and_then(|t| t.fresh(now)),
            throttle_current: self.throttle_current.and_then(|t| t.fresh(now)),
            throttle_rpm: self.throttle_rpm.and_then(|t| t.fresh(now)),
            throttle_rel: self.throttle_rel.and_then(|t| t.fresh(now)),
            throttle_pos: self.throttle_pos.and_then(|t| t.fresh(now)),
            throttle_errors: if throttle_err_fresh { self.throttle_errors } else { 0 },
            mppts,
            heights,
            hottest_mppt: hottest_mppt.clone(),
            hottest_battery: hottest_battery.clone(),
            warnings: WarningsView {
                // Numeric codes here are battery_state_num/charge_state_num/
                // discharge_state_num's encoding of BatteryState::On (6),
                // ChargeState::FetOn (3), and DischargeState::On (3).
                battery_state: batt_state.is_some_and(|s| s != 6),
                charge_fet: chg_state.is_some_and(|s| s != 3),
                discharge_fet: dis_state.is_some_and(|s| s != 3),
                soc_low: soc.is_some_and(|s| s < 15.0),
                motor_hot: motor_c.is_some_and(|c| c > 50.0),
                fet_hot: fet_c.is_some_and(|c| c > 70.0),
                mppt_hot: hottest_mppt.as_ref().is_some_and(|(_, c)| *c > 80.0),
                battery_hot: hottest_battery.as_ref().is_some_and(|(_, c)| *c > 45.0),
                throttle: throttle_err_fresh && self.throttle_errors != 0,
            },
            bus: BusHealthView {
                frames_per_sec: self.frame_times.len() as f32,
                frames_total: self.frames_total,
                frames_unknown: self.frames_unknown,
                // eoi-can-decoder doesn't distinguish "malformed frame" from
                // "unrecognized ID" the way the old decoder's DecodeError did — both
                // just return None — so this stays 0 rather than double-count into
                // frames_unknown.
                decode_errors: 0,
            },
        }
    }
}

fn apply_mppt_info(slot: &mut MpptSlot, info: &MpptInfo, now: Instant) {
    let apply_channel = |ch_slot: &mut MpptChannelSlot, ch: &MpptChannel| match ch {
        MpptChannel::Power(p) => {
            ch_slot.vin = Some(Timed { value: p.voltage_in, at: now });
            ch_slot.iin = Some(Timed { value: p.current_in, at: now });
        }
        // Duty cycle/algorithm state has no Snapshot field yet.
        MpptChannel::State(_) => {}
    };
    match info {
        MpptInfo::Channel0(ch) => apply_channel(&mut slot.channels[0], ch),
        MpptInfo::Channel1(ch) => apply_channel(&mut slot.channels[1], ch),
        MpptInfo::Channel2(ch) => apply_channel(&mut slot.channels[2], ch),
        MpptInfo::Channel3(ch) => apply_channel(&mut slot.channels[3], ch),
        // No channel index to attribute this to; drop it, same as the old decoder did.
        MpptInfo::ChannelUnknown(_) => {}
        MpptInfo::Power(p) => {
            slot.vout = Some(Timed { value: p.voltage_out, at: now });
            slot.iout = Some(Timed { value: p.current_out, at: now });
        }
        MpptInfo::Status(s) => {
            slot.celsius = Some(Timed { value: s.temperature as f32, at: now });
            slot.state = Some(Timed { value: s.state as u32, at: now });
            slot.flags = (s.pwm_enabled as u32) | ((s.switch_on as u32) << 1);
        }
    }
}

fn apply_gan_packet(slot: &mut MpptSlot, packet: &GanMpptPacket, now: Instant) {
    match packet {
        GanMpptPacket::Power(p) => {
            slot.channels[0].vin = Some(Timed { value: p.input_voltage, at: now });
            slot.channels[0].iin = Some(Timed { value: p.input_current, at: now });
            slot.vout = Some(Timed { value: p.output_voltage, at: now });
            slot.iout = Some(Timed { value: p.output_current, at: now });
        }
        GanMpptPacket::Status(s) => {
            slot.celsius = Some(Timed { value: s.board_temp as f32, at: now });
            slot.heat_sink_celsius = Some(Timed { value: s.heat_sink_temp as f32, at: now });
            slot.state = Some(Timed { value: gan_mode_num(&s.mode), at: now });
            slot.flags = gan_fault_num(&s.fault);
        }
        // No Snapshot field for a sweep trace.
        GanMpptPacket::SweepData(_) => {}
    }
}

/// Reduces up to four independently-tracked input channels to the single vin/iin pair
/// `MpptView` has room for: total current across fresh channels, and a
/// current-weighted average voltage (falling back to the first fresh channel's voltage
/// if every fresh channel happens to read zero current). A deliberate simplification,
/// not a rediscovery of the old decoder's "last channel wins" behaviour — see the
/// module doc comment.
fn mppt_view(gan: bool, id: u32, slot: &MpptSlot, now: Instant) -> MpptView {
    let mut total_iin = 0f32;
    let mut weighted_vin = 0f32;
    let mut any_fresh = false;
    let mut first_vin = None;
    for ch in &slot.channels {
        let vin = ch.vin.and_then(|t| t.fresh(now));
        let iin = ch.iin.and_then(|t| t.fresh(now));
        if let Some(v) = vin {
            first_vin.get_or_insert(v);
        }
        if let (Some(v), Some(i)) = (vin, iin) {
            any_fresh = true;
            total_iin += i;
            weighted_vin += v * i;
        }
    }
    let iin = any_fresh.then_some(total_iin);
    let vin = if any_fresh && total_iin.abs() > f32::EPSILON {
        Some(weighted_vin / total_iin)
    } else {
        first_vin
    };

    let celsius = slot.celsius.and_then(|t| t.fresh(now));
    let heat_sink = slot.heat_sink_celsius.and_then(|t| t.fresh(now));
    let hottest_celsius = match (celsius, heat_sink) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    MpptView {
        label: if gan { format!("GaN {id}") } else { format!("MPPT {id}") },
        id,
        gan,
        vin,
        iin,
        vout: slot.vout.and_then(|t| t.fresh(now)),
        iout: slot.iout.and_then(|t| t.fresh(now)),
        celsius: hottest_celsius,
        state: slot.state.and_then(|t| t.fresh(now)),
        flags: slot.flags,
    }
}

fn motor_ntc_quality(
    status: &eoi_can_decoder::MotorNtcStatus,
    has_temp: bool,
) -> Quality {
    if !has_temp || status.sensor_open || status.sensor_short || status.acquisition_error {
        Quality::Absent
    } else if status.out_of_range {
        Quality::Clamped
    } else if status.settling {
        Quality::Settling
    } else {
        Quality::Ok
    }
}

fn throttle_error_bits(e: &ThrottleErrors) -> u32 {
    let twi: u32 = match e.twi {
        ThrottleTwiErrors::NoError => 0,
        ThrottleTwiErrors::BusFault => 1,
        ThrottleTwiErrors::BusCaptureTimeout => 2,
        ThrottleTwiErrors::SlaveResponseTimeout => 3,
        ThrottleTwiErrors::SlaveNotReady => 4,
        ThrottleTwiErrors::SlaveNAK => 5,
        ThrottleTwiErrors::Unknown => 6,
    };
    (twi & 0b111)
        | ((e.no_eeprom as u32) << 3)
        | ((e.gain_clipping as u32) << 4)
        | ((e.gain_invalid as u32) << 5)
        | ((e.deadman_missing as u32) << 6)
        | ((e.impedance_high as u32) << 7)
}

fn battery_state_num(s: &BatteryState) -> u32 {
    match s {
        BatteryState::Init => 0,
        BatteryState::Sleep => 1,
        BatteryState::WaitingForStartup => 2,
        BatteryState::Idle => 3,
        BatteryState::OnlyCharge => 4,
        BatteryState::OnlyDischarge => 5,
        BatteryState::On => 6,
        BatteryState::Unknown => 7,
    }
}

fn charge_state_num(s: &ChargeState) -> u32 {
    match s {
        ChargeState::Init => 0,
        ChargeState::Idle => 1,
        ChargeState::RelayOn => 2,
        ChargeState::FetOn => 3,
        ChargeState::Error => 4,
        ChargeState::FetOff => 5,
        ChargeState::Unknown => 6,
    }
}

fn discharge_state_num(s: &DischargeState) -> u32 {
    match s {
        DischargeState::Init => 0,
        DischargeState::Idle => 1,
        DischargeState::PreChargeOn => 2,
        DischargeState::On => 3,
        DischargeState::PreChargeTimeout => 4,
        DischargeState::Error => 5,
        DischargeState::Unknown => 6,
    }
}

fn height_state_num(s: &HeightSensorState) -> u32 {
    match s {
        HeightSensorState::NotPluggedIn => 0,
        HeightSensorState::ModbusError => 1,
        HeightSensorState::Operational => 2,
        HeightSensorState::Unknown => 3,
    }
}

fn servo_state_num(s: &ServoState) -> u32 {
    match s {
        ServoState::Uninitialized => 0,
        ServoState::Operational => 1,
        ServoState::Homing => 2,
        ServoState::FailSafe => 3,
        ServoState::Fault => 4,
        ServoState::Unknown => 5,
    }
}

fn servo_fault_num(s: &ServoFaultCause) -> u32 {
    match s {
        ServoFaultCause::None => 0,
        ServoFaultCause::StallDuringMove => 1,
        ServoFaultCause::HomingTimeout => 2,
        ServoFaultCause::DriverNoUartResponse => 3,
        ServoFaultCause::DriverError => 4,
        ServoFaultCause::DriverOpenLoad => 5,
        ServoFaultCause::Unknown => 6,
    }
}

fn gan_mode_num(m: &GanPhaseMode) -> u32 {
    match m {
        GanPhaseMode::None => 0,
        GanPhaseMode::Civ => 1,
        GanPhaseMode::Cic => 2,
        GanPhaseMode::MinInputCurrent => 3,
        GanPhaseMode::Cov => 4,
        GanPhaseMode::Coc => 5,
        GanPhaseMode::TemperatureDerating => 6,
        GanPhaseMode::Fault => 7,
        GanPhaseMode::Unknown => 8,
    }
}

fn gan_fault_num(f: &GanPhaseFault) -> u32 {
    match f {
        GanPhaseFault::Ok => 0,
        GanPhaseFault::ConfigError => 1,
        GanPhaseFault::InputOverVoltage => 2,
        GanPhaseFault::OutputOverVoltage => 3,
        GanPhaseFault::OutputOverCurrent => 4,
        GanPhaseFault::InputOverCurrent => 5,
        GanPhaseFault::InputUnderCurrent => 6,
        GanPhaseFault::PhaseOverCurrent => 7,
        GanPhaseFault::GeneralFault => 8,
        GanPhaseFault::Unknown => 9,
    }
}

fn to_can_id(raw: u32) -> Id {
    if raw <= 0x7FF {
        Id::Standard(StandardId::new(raw as u16).expect("raw <= 0x7FF fits StandardId"))
    } else {
        Id::Extended(ExtendedId::new(raw).expect("raw fits 29 bits"))
    }
}

/// Decodes one raw `(id, data)` CAN frame and applies it to `state`. `id` doesn't need
/// to carry standard/extended framing information — `eoi-can-decoder` matches purely
/// on the numeric ID either way, so `to_can_id` just picks whichever framing fits the
/// value, matching how the old decoder ignored the distinction entirely.
pub fn apply_frame(state: &mut LiveState, id: u32, data: &[u8]) {
    let frame = CanFrame::from_encoded(to_can_id(id), data);
    match parse_eoi_can_data(&frame) {
        Some(update) => {
            state.apply(update, Instant::now());
        }
        None => {
            tracing::debug!(id = format_args!("{id:#x}"), "unknown or malformed CAN frame");
            state.note_unknown(Instant::now());
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotView {
    pub boat_uptime_ms: Option<u64>,
    pub net_w: Option<f32>,
    pub in_w: Option<f32>,
    pub out_w: Option<f32>,
    pub pack_v: Option<f32>,
    pub pack_a: Option<f32>,
    pub peri_a: Option<f32>,
    pub charge_a: Option<f32>,
    pub discharge_a: Option<f32>,
    pub soc_pct: Option<f32>,
    pub stack_v: Option<f32>,
    pub cell_v: Vec<f32>,
    pub batt_temps: Vec<(String, Option<f32>, Quality)>,
    pub batt_state: Option<u32>,
    pub charge_state: Option<u32>,
    pub discharge_state: Option<u32>,
    pub error_flags: u32,
    pub balancing_flags: u32,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub speed_kmh: Option<f32>,
    pub heading_deg: Option<f32>,
    pub gnss_fix: Option<u32>,
    pub gnss_sats: Option<u32>,
    pub gnss_sats_used: Option<u32>,
    pub rpm: Option<f32>,
    pub motor_current_a: Option<f32>,
    pub motor_duty: Option<f32>,
    pub tacho: Option<i32>,
    pub fet_celsius: Option<f32>,
    pub motor_celsius: Option<f32>,
    pub motor_temp_quality: Quality,
    pub ah_used: Option<f32>,
    pub ah_gen: Option<f32>,
    pub wh_used: Option<f32>,
    pub wh_gen: Option<f32>,
    pub input_v: Option<f32>,
    pub rudder_setpoint: Option<u32>,
    pub rudder_actual: Option<u32>,
    pub steering_deg: Option<f32>,
    pub rudder_state: Option<u32>,
    pub rudder_fault: Option<u32>,
    pub rudder_ctl_c: Option<f32>,
    pub height_ctl_c: Option<f32>,
    pub flow_in: Option<f32>,
    pub flow_out: Option<f32>,
    pub throttle_duty: Option<f32>,
    pub throttle_current: Option<f32>,
    pub throttle_rpm: Option<f32>,
    pub throttle_rel: Option<f32>,
    pub throttle_pos: Option<f32>,
    pub throttle_errors: u32,
    pub mppts: Vec<MpptView>,
    pub heights: Vec<HeightView>,
    pub hottest_mppt: Option<(String, f32)>,
    pub hottest_battery: Option<(String, f32)>,
    pub warnings: WarningsView,
    pub bus: BusHealthView,
}

#[derive(Debug, Clone)]
pub struct MpptView {
    pub label: String,
    pub id: u32,
    pub gan: bool,
    pub vin: Option<f32>,
    pub iin: Option<f32>,
    pub vout: Option<f32>,
    pub iout: Option<f32>,
    pub celsius: Option<f32>,
    pub state: Option<u32>,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct HeightView {
    pub label: String,
    pub state: Option<u32>,
    pub height: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct WarningsView {
    pub battery_state: bool,
    pub charge_fet: bool,
    pub discharge_fet: bool,
    pub soc_low: bool,
    pub motor_hot: bool,
    pub fet_hot: bool,
    pub mppt_hot: bool,
    pub battery_hot: bool,
    pub throttle: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BusHealthView {
    pub frames_per_sec: f32,
    pub frames_total: u64,
    pub frames_unknown: u64,
    pub decode_errors: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_fields_drop_after_five_seconds() {
        let t0 = Instant::now();
        let mut state = LiveState::new();
        let mut soc = [0u8; 8];
        soc[0..2].copy_from_slice(&9200u16.to_le_bytes()); // 92.00 %
        apply_frame(&mut state, 0x102, &soc);
        std::thread::sleep(Duration::from_millis(1));
        assert_eq!(state.view(t0 + Duration::from_secs(1)).soc_pct, Some(92.0));
        assert_eq!(state.view(t0 + Duration::from_secs(6)).soc_pct, None);
    }

    #[test]
    fn motor_temp_never_comes_from_vesc_status_4() {
        let mut state = LiveState::new();
        let mut vesc = [0u8; 8];
        vesc[0..2].copy_from_slice(&420i16.to_be_bytes()); // 42.0 FET
        vesc[2..4].copy_from_slice(&(-32000i16).to_be_bytes()); // nonsense motor temp
        apply_frame(&mut state, 0x1009, &vesc);

        let mut ntc = [0u8; 4];
        ntc[0..2].copy_from_slice(&255i16.to_le_bytes()); // 25.5 C
        apply_frame(&mut state, 0x219, &ntc);

        let view = state.view(Instant::now());
        assert_eq!(view.motor_celsius, Some(25.5));
        assert_eq!(view.fet_celsius, Some(42.0));
    }

    #[test]
    fn mppt_channels_stay_independent() {
        let mut state = LiveState::new();
        // MPPT node 0, channel 0 power (field 0) then channel 1 power (field 2).
        let mut ch0 = [0u8; 8];
        ch0[0..4].copy_from_slice(&20.0f32.to_le_bytes());
        ch0[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        apply_frame(&mut state, 0x700, &ch0);

        let mut ch1 = [0u8; 8];
        ch1[0..4].copy_from_slice(&18.0f32.to_le_bytes());
        ch1[4..8].copy_from_slice(&2.0f32.to_le_bytes());
        apply_frame(&mut state, 0x702, &ch1);

        let view = state.view(Instant::now());
        let mppt = view.mppts.iter().find(|m| m.id == 0).unwrap();
        // Both channels contribute: total current 3A, current-weighted voltage.
        assert_eq!(mppt.iin, Some(3.0));
        assert!((mppt.vin.unwrap() - ((20.0 * 1.0 + 18.0 * 2.0) / 3.0)).abs() < 1e-4);
    }
}
