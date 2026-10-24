//! What the display knows: the latest value of each signal with the time it arrived,
//! a [`View`] of the fresh ones, and a [`History`] of views for the strip chart.
//!
//! Time is seconds on any monotonic clock the caller chooses (wall time live, log time
//! when replaying), passed in rather than read, so a replay renders exactly what the
//! boat would have shown.

use std::collections::VecDeque;

use eoi_can_decoder::{
    EoiCanData, HeightSensorData, HeightSensorState, HydrofoilData, RudderControllerData,
    ServoData, ServoState,
};

use crate::config::BoatConfig;

/// A value older than this is not shown. The 10 Hz frames (`0x250`-`0x252`, `0x275`,
/// `0x020`) tolerate four missed frames.
pub const MAX_AGE_S: f64 = 0.5;
/// For `0x270`, which the flight controller sends at 2 Hz.
pub const MAX_AGE_DEBUG_S: f64 = 1.5;

/// `0x251` byte 6.
pub const MODE_MANUAL: u8 = 0;

#[derive(Debug, Clone, Copy)]
struct Timed<T> {
    value: T,
    at: f64,
}

fn timed<T>(value: T, at: f64) -> Option<Timed<T>> {
    Some(Timed { value, at })
}

fn fresh<T: Copy>(v: &Option<Timed<T>>, now: f64, max_age: f64) -> Option<T> {
    v.filter(|t| now - t.at <= max_age && now >= t.at - 1e-6)
        .map(|t| t.value)
}

#[derive(Debug, Clone, Copy)]
struct Rear {
    operational: bool,
    setpoint: u16,
    position: u16,
}

/// The latest of everything the display draws. Feed it with [`FoilState::ingest`].
#[derive(Debug, Default, Clone)]
pub struct FoilState {
    attitude: Option<Timed<(f32, f32)>>,
    demand: Option<Timed<(Option<f32>, Option<f32>)>>,
    /// `0x271`'s pitch demand: the 2 Hz stand-in for logs from before `0x275`.
    loop_demand: Option<Timed<Option<f32>>>,
    /// `0x251` EKF height, which is the height the loop controls on (`HYDC.H`).
    height_m: Option<Timed<Option<f32>>>,
    mode: Option<Timed<u8>>,
    target_m: Option<Timed<Option<f32>>>,
    elevons_us: Option<Timed<(u16, u16)>>,
    rear: Option<Timed<Rear>>,
    sensor_fl_m: Option<Timed<Option<f32>>>,
    sensor_fr_m: Option<Timed<Option<f32>>>,
}

/// Everything one frame draws, with `None` for anything missing or stale.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct View {
    pub roll_deg: Option<f32>,
    pub pitch_deg: Option<f32>,
    pub des_roll_deg: Option<f32>,
    pub des_pitch_deg: Option<f32>,
    pub height_m: Option<f32>,
    pub target_m: Option<f32>,
    pub fl_aoa_deg: Option<f32>,
    pub fr_aoa_deg: Option<f32>,
    pub fl_stop: bool,
    pub fr_stop: bool,
    pub rear_aoa_deg: Option<f32>,
    pub rear_cmd_aoa_deg: Option<f32>,
    pub fl_sensor_m: Option<f32>,
    pub fr_sensor_m: Option<f32>,
    pub mode: Option<u8>,
}

fn sensor_m(s: &eoi_can_decoder::HeightSensorStatus) -> Option<f32> {
    (s.state == HeightSensorState::Operational).then(|| s.value as f32 / 1000.0)
}

impl FoilState {
    pub fn ingest(&mut self, data: &EoiCanData, now: f64) {
        match data {
            EoiCanData::Hydrofoil(h) => match h {
                HydrofoilData::Attitude(a) => self.attitude = timed((a.roll_deg, a.pitch_deg), now),
                HydrofoilData::Demand(d) => self.demand = timed((d.roll_deg, d.pitch_deg), now),
                HydrofoilData::State(s) => {
                    self.height_m = timed(s.ekf_height_mm.map(|mm| mm / 1000.0), now);
                    self.mode = timed(s.mode, now);
                }
                HydrofoilData::Elevons(e) => self.elevons_us = timed((e.left_us, e.right_us), now),
                HydrofoilData::DebugHeightLoop(l) => self.loop_demand = timed(l.demand_deg, now),
                HydrofoilData::DebugPhase(p) => {
                    self.target_m = timed(p.target_mm.map(|mm| mm / 1000.0), now)
                }
                _ => {}
            },
            EoiCanData::RudderController(RudderControllerData::Servo(ServoData::Status(s))) => {
                self.rear = timed(
                    Rear {
                        operational: s.state == ServoState::Operational,
                        setpoint: s.setpoint,
                        position: s.actual_position,
                    },
                    now,
                )
            }
            EoiCanData::HeightSensors(HeightSensorData::FrontLeft(s)) => {
                self.sensor_fl_m = timed(sensor_m(s), now)
            }
            EoiCanData::HeightSensors(HeightSensorData::FrontRight(s)) => {
                self.sensor_fr_m = timed(sensor_m(s), now)
            }
            _ => {}
        }
    }

    pub fn view(&self, now: f64, cfg: &BoatConfig) -> View {
        let attitude = fresh(&self.attitude, now, MAX_AGE_S);
        let demand = fresh(&self.demand, now, MAX_AGE_S);
        let mode = fresh(&self.mode, now, MAX_AGE_S);
        // Without 0x275 (a script from before 2026-10-23), fall back to the height
        // loop's demand in 0x271. It only covers height mode: it reads 0 while disabled
        // (so MANUAL shows none) and in roll test, which has no 0x275 stand-in at all.
        let des_pitch_deg = match demand {
            Some(d) => d.1,
            None => fresh(&self.loop_demand, now, MAX_AGE_DEBUG_S)
                .flatten()
                .filter(|_| mode != Some(MODE_MANUAL)),
        };
        // PWM 0 = the flight controller had no output to report.
        let elevons = fresh(&self.elevons_us, now, MAX_AGE_S);
        let fl_us = elevons.map(|e| e.0).filter(|&us| us != 0).map(f32::from);
        let fr_us = elevons.map(|e| e.1).filter(|&us| us != 0).map(f32::from);
        let wings = &cfg.front_wings;
        // The stepper reads 2000 (home) whenever it is not Operational, which would
        // draw as a real angle: show nothing instead.
        let rear = fresh(&self.rear, now, MAX_AGE_S).filter(|r| r.operational);

        View {
            roll_deg: attitude.map(|a| a.0),
            pitch_deg: attitude.map(|a| a.1),
            des_roll_deg: demand.and_then(|d| d.0),
            des_pitch_deg,
            height_m: fresh(&self.height_m, now, MAX_AGE_S).flatten(),
            target_m: fresh(&self.target_m, now, MAX_AGE_DEBUG_S).flatten(),
            fl_aoa_deg: fl_us.map(|us| wings.fl.aoa(us)),
            fr_aoa_deg: fr_us.map(|us| wings.fr.aoa(us)),
            fl_stop: fl_us.is_some_and(|us| wings.fl.at_stop(us, wings.stop_tol_us)),
            fr_stop: fr_us.is_some_and(|us| wings.fr.at_stop(us, wings.stop_tol_us)),
            rear_aoa_deg: rear.map(|r| cfg.rear_wing.aoa(f32::from(r.position))),
            rear_cmd_aoa_deg: rear.map(|r| cfg.rear_wing.aoa(f32::from(r.setpoint))),
            fl_sensor_m: fresh(&self.sensor_fl_m, now, MAX_AGE_S).flatten(),
            fr_sensor_m: fresh(&self.sensor_fr_m, now, MAX_AGE_S).flatten(),
            mode,
        }
    }
}

/// [`View`]s at a fixed rate over the trailing [`History::SPAN_S`], for the strip chart.
#[derive(Debug, Clone)]
pub struct History {
    samples: VecDeque<(f64, View)>,
    next_at: Option<f64>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub const SPAN_S: f64 = 10.0;
    pub const PERIOD_S: f64 = 0.04;

    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity((Self::SPAN_S / Self::PERIOD_S) as usize + 2),
            next_at: None,
        }
    }

    /// Samples `state` at every tick up to and including `now`. Call it before applying
    /// a frame received at `now`, so each tick sees what was current at that instant.
    pub fn advance(&mut self, state: &FoilState, cfg: &BoatConfig, now: f64) {
        let mut t = self.next_at.unwrap_or(now);
        // A long gap (a paused replay, a suspended laptop) would otherwise be sampled
        // tick by tick; anything before the window is dropped anyway.
        if now - t > Self::SPAN_S {
            t = now - Self::SPAN_S;
        }
        while t <= now {
            self.samples.push_back((t, state.view(t, cfg)));
            t += Self::PERIOD_S;
        }
        self.next_at = Some(t);
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now - at > Self::SPAN_S + Self::PERIOD_S)
        {
            self.samples.pop_front();
        }
    }

    /// Oldest first, as (seconds before `now`, view).
    pub fn iter(&self, now: f64) -> impl Iterator<Item = (f64, &View)> {
        self.samples.iter().map(move |(at, v)| (now - at, v))
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// The state, its history and the config they are read with: what a source feeds and
/// a frame is drawn from.
#[derive(Debug, Clone, Default)]
pub struct Model {
    pub cfg: BoatConfig,
    pub state: FoilState,
    pub history: History,
}

impl Model {
    pub fn new(cfg: BoatConfig) -> Self {
        Self {
            cfg,
            state: FoilState::default(),
            history: History::new(),
        }
    }

    pub fn ingest(&mut self, data: &EoiCanData, now: f64) {
        self.history.advance(&self.state, &self.cfg, now);
        self.state.ingest(data, now);
    }

    /// Brings the history up to `now` and returns what to draw.
    pub fn frame(&mut self, now: f64) -> View {
        self.history.advance(&self.state, &self.cfg, now);
        self.state.view(now, &self.cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_can::{Id, StandardId};
    use eoi_can_decoder::{can_frame::CanFrame, parse_eoi_can_data};

    fn data(id: u16, bytes: &[u8]) -> EoiCanData {
        let frame = CanFrame::from_encoded(Id::Standard(StandardId::new(id).unwrap()), bytes);
        parse_eoi_can_data(&frame).expect("decodes")
    }

    #[test]
    fn values_go_stale() {
        let cfg = BoatConfig::default();
        let mut s = FoilState::default();
        // roll +3.00, pitch -1.50
        s.ingest(&data(0x250, &[0x2C, 0x01, 0x6A, 0xFF, 0, 0]), 10.0);
        let v = s.view(10.4, &cfg);
        assert_eq!((v.roll_deg, v.pitch_deg), (Some(3.0), Some(-1.5)));
        assert_eq!(s.view(10.6, &cfg).roll_deg, None);
    }

    #[test]
    fn wings_from_pwm_and_stepper() {
        let cfg = BoatConfig::default();
        let mut s = FoilState::default();
        // FL at its low stop (1435 us), FR at 1420 us.
        s.ingest(&data(0x252, &[0x9B, 0x05, 0x8C, 0x05]), 1.0);
        // Rear Operational, setpoint 1500, position 1750.
        s.ingest(&data(0x020, &[1, 0xDC, 0x05, 0xD6, 0x06, 0]), 1.0);
        let v = s.view(1.0, &cfg);
        assert_eq!(v.fl_aoa_deg, Some(-7.5));
        assert!(v.fl_stop && !v.fr_stop);
        assert!((v.fr_aoa_deg.unwrap() - 1.0).abs() < 1e-4);
        assert!((v.rear_aoa_deg.unwrap() - 0.0).abs() < 1e-4);
        assert!((v.rear_cmd_aoa_deg.unwrap() - 3.0).abs() < 1e-4);

        // Homing: the position reads 2000 but is not an angle.
        s.ingest(&data(0x020, &[2, 0xD0, 0x07, 0xD0, 0x07, 0]), 1.1);
        assert_eq!(s.view(1.1, &cfg).rear_aoa_deg, None);
    }

    #[test]
    fn demand_none_is_not_zero() {
        let cfg = BoatConfig::default();
        let mut s = FoilState::default();
        s.ingest(
            &data(0x275, &[0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80]),
            0.0,
        );
        let v = s.view(0.0, &cfg);
        assert_eq!((v.des_roll_deg, v.des_pitch_deg), (None, None));
        s.ingest(
            &data(0x275, &[0x2C, 0x01, 0xC8, 0x00, 0x00, 0x80, 0x00, 0x80]),
            0.1,
        );
        let v = s.view(0.1, &cfg);
        assert_eq!((v.des_roll_deg, v.des_pitch_deg), (Some(3.0), Some(2.0)));
    }

    #[test]
    fn old_logs_get_the_pitch_demand_from_0x271() {
        let cfg = BoatConfig::default();
        let mut s = FoilState::default();
        // FBWA, then the height loop demanding +0.75 deg.
        s.ingest(&data(0x251, &[0xFF, 0xFF, 0x2C, 0x01, 0, 0, 5, 4]), 0.0);
        s.ingest(&data(0x271, &[0, 0, 0, 0, 0, 0, 0x4B, 0x00]), 0.0);
        assert_eq!(s.view(0.4, &cfg).des_pitch_deg, Some(0.75));
        // 0x275, once it arrives, wins, and its "none" means none.
        s.ingest(
            &data(0x275, &[0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80]),
            0.4,
        );
        assert_eq!(s.view(0.4, &cfg).des_pitch_deg, None);
        // MANUAL: the loop's 0 is not a demand.
        let mut s = FoilState::default();
        s.ingest(&data(0x251, &[0xFF, 0xFF, 0x2C, 0x01, 0, 0, 0, 4]), 0.0);
        s.ingest(&data(0x271, &[0, 0, 0, 0, 0, 0, 0, 0]), 0.0);
        assert_eq!(s.view(0.1, &cfg).des_pitch_deg, None);
    }

    #[test]
    fn target_survives_between_2hz_frames() {
        let cfg = BoatConfig::default();
        let mut s = FoilState::default();
        // Phase 4, target 300 mm.
        s.ingest(&data(0x270, &[4, 0, 0x2C, 0x01, 0x2C, 0x01, 0, 0]), 0.0);
        assert_eq!(s.view(0.9, &cfg).target_m, Some(0.3));
        assert_eq!(s.view(1.6, &cfg).target_m, None);
    }

    #[test]
    fn history_keeps_ten_seconds_at_a_fixed_rate() {
        let mut m = Model::default();
        for k in 0..=300 {
            m.ingest(&data(0x250, &[0, 0, 0, 0, 0, 0]), k as f64 * 0.1);
        }
        m.frame(30.0);
        let ages: Vec<f64> = m.history.iter(30.0).map(|(age, _)| age).collect();
        assert!(ages.first().unwrap() <= &(History::SPAN_S + History::PERIOD_S));
        assert!(ages.last().unwrap().abs() < History::PERIOD_S);
        assert!((250..=253).contains(&ages.len()), "{}", ages.len());
        assert!(
            ages.windows(2)
                .all(|w| (w[0] - w[1] - History::PERIOD_S).abs() < 1e-6)
        );
    }
}
