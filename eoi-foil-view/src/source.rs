//! Where frames come from: a live SocketCAN interface, a candump log, or a built-in
//! demo. All three hand raw (id, bytes) to the same decoder, so the demo and a replay
//! exercise exactly the path the boat does.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use embedded_can::{ExtendedId, Id, StandardId};
use eoi_can_decoder::{can_frame::CanFrame, parse_eoi_can_data};

use crate::state::Model;

pub struct RawFrame {
    pub id: Id,
    pub data: Vec<u8>,
}

impl RawFrame {
    pub fn standard(id: u16, data: &[u8]) -> Self {
        Self {
            id: Id::Standard(StandardId::new(id).expect("11-bit id")),
            data: data.to_vec(),
        }
    }

    pub fn extended(id: u32, data: &[u8]) -> Self {
        Self {
            id: Id::Extended(ExtendedId::new(id).expect("29-bit id")),
            data: data.to_vec(),
        }
    }
}

/// Decodes `frame` and applies it at `now`; frames the decoder does not know, and
/// frames longer than a classic CAN frame, are ignored.
pub fn apply(model: &mut Model, frame: &RawFrame, now: f64) {
    if frame.data.len() > 8 {
        return;
    }
    let frame = CanFrame::from_encoded(frame.id, &frame.data);
    if let Some(data) = parse_eoi_can_data(&frame) {
        model.ingest(&data, now);
    }
}

// ---- candump -l ------------------------------------------------------------------------

/// One `candump -l` line: `(1778835265.757050) can0 00001009#0095022A00000239`.
/// An id of more than three hex digits is extended, as candump writes it.
pub fn parse_candump_line(line: &str) -> Option<(f64, RawFrame)> {
    let line = line.trim();
    let rest = line.strip_prefix('(')?;
    let (ts, rest) = rest.split_once(')')?;
    let ts: f64 = ts.trim().parse().ok()?;
    let mut parts = rest.split_whitespace();
    let _iface = parts.next()?;
    let (id_s, data_s) = parts.next()?.split_once('#')?;
    let id = u32::from_str_radix(id_s, 16).ok()?;
    if data_s.len() % 2 != 0 || data_s.starts_with('R') {
        return None;
    }
    let data = (0..data_s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&data_s[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let frame = if id_s.len() > 3 {
        RawFrame::extended(id, &data)
    } else {
        RawFrame::standard(u16::try_from(id).ok()?, &data)
    };
    Some((ts, frame))
}

/// Reads a candump log as (seconds since `from` into the log, frame), skipping
/// everything before `from` seconds.
pub fn candump_frames(
    path: &Path,
    from: f64,
) -> std::io::Result<impl Iterator<Item = (f64, RawFrame)>> {
    let reader = BufReader::new(std::fs::File::open(path)?);
    let mut start = None;
    Ok(reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| parse_candump_line(&line))
        .filter_map(move |(ts, frame)| {
            let t = ts - *start.get_or_insert(ts) - from;
            (t >= 0.0).then_some((t, frame))
        }))
}

// ---- demo -------------------------------------------------------------------------------

fn le16(v: i32) -> [u8; 2] {
    (v as u16).to_le_bytes()
}

fn cdeg(deg: Option<f32>) -> [u8; 2] {
    le16(deg.map_or(i32::from(i16::MIN), |d| (d * 100.0).round() as i32))
}

fn smooth(t: f32, t0: f32, t1: f32) -> f32 {
    let x = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The frames the boat would send at `t` seconds into a 60 s demo run: floating in
/// MANUAL, a takeoff, foiling through rolls (the front-left wing reaching its stop
/// for a moment around 31 s), and back down.
pub fn demo_frames(t: f64) -> Vec<RawFrame> {
    let t = t.rem_euclid(60.0) as f32;
    let fbwa = (4.0..52.0).contains(&t);
    let up = smooth(t, 6.0, 11.0) * (1.0 - smooth(t, 47.0, 52.0));
    let height = 0.114 + up * (0.186 + 0.025 * (t * 1.3).sin() + 0.01 * (t * 4.1).sin());
    let target = 0.30;
    let roll = up * (3.5 * (t * 0.4).sin() + 0.6 * (t * 2.3).sin());
    let des_roll = up * 3.2 * (t * 0.4 - 0.25).sin();
    let pitch = 0.3
        + up * (0.9 + 0.7 * (t * 1.3 + 0.6).sin())
        + 1.4 * smooth(t, 4.0, 7.0) * (1.0 - smooth(t, 7.0, 10.0));
    let des_pitch = (pitch + 0.3 * (t * 1.3 + 1.2).sin()).clamp(-3.0, 2.0);

    // Front wings: cruise trim plus pitch demand, roll differential. FL hits its
    // upper stop (1730 us) for a moment around 31 s.
    let (fl_deg, fr_deg) = if fbwa {
        let common = 2.8 + 1.6 * (des_pitch - pitch);
        let diff = 1.2 * (des_roll - roll);
        let spike = 8.0 * smooth(t, 30.0, 30.6) * (1.0 - smooth(t, 31.4, 32.0));
        (common - diff + spike, common + diff)
    } else {
        (0.0, 0.0)
    };
    let fl_us = (1435.0 + (fl_deg + 7.5) / 17.0 * 295.0).clamp(1435.0, 1730.0);
    let fr_us = (1300.0 + (9.5 - fr_deg) / 17.0 * 240.0).clamp(1300.0, 1540.0);

    // Rear: speed-scheduled command, the stepper lagging it by a few units.
    let rear_cmd_deg = if fbwa {
        1.5 + 2.0 * up + 0.4 * (t * 0.5).sin()
    } else {
        -3.0
    };
    let rear_sp = 2000.0 - (rear_cmd_deg + 3.0) / 0.012;
    let rear_pos = rear_sp + 20.0 * (t * 0.5 + 1.0).cos() * up;

    // Each strut sensor sees the ride height moved by the roll over half a metre.
    let lever = 0.5 * roll.to_radians().sin();
    let noise = 0.006 * (t * 37.0).sin();
    let fl_mm = ((height - lever + noise) * 1000.0) as i32;
    let fr_mm = ((height + lever - noise) * 1000.0) as i32;

    let [rl, rh] = cdeg(Some(roll));
    let [pl, ph] = cdeg(Some(pitch));
    let (d_roll, d_pitch) = if fbwa {
        (Some(des_roll), Some(des_pitch))
    } else {
        (None, None)
    };
    let [drl, drh] = cdeg(d_roll);
    let [dpl, dph] = cdeg(d_pitch);
    let [hl, hh] = le16((height * 1000.0) as i32);
    let [tl, th] = le16((target * 1000.0) as i32);
    let [fll, flh] = le16(fl_us as i32);
    let [frl, frh] = le16(fr_us as i32);
    let [spl, sph] = le16(rear_sp as i32);
    let [pol, poh] = le16(rear_pos as i32);
    let [sfl, sfh] = le16(fl_mm);
    let [srl, srh] = le16(fr_mm);
    let mode = if fbwa { 5 } else { 0 };

    vec![
        RawFrame::standard(0x250, &[rl, rh, pl, ph, 0x10, 0x27]),
        RawFrame::standard(0x251, &[0xFF, 0xFF, hl, hh, 0x31, 0x0F, mode, 4]),
        RawFrame::standard(0x252, &[fll, flh, frl, frh]),
        RawFrame::standard(0x275, &[drl, drh, dpl, dph, 0x00, 0x80, 0x00, 0x80]),
        RawFrame::standard(0x270, &[4, 0, hl, hh, tl, th, 0, 0]),
        RawFrame::standard(0x020, &[1, spl, sph, pol, poh, 0]),
        RawFrame::standard(0x011, &[2, sfl, sfh]),
        RawFrame::standard(0x012, &[2, srl, srh]),
    ]
}

/// Feeds the demo into `model` from `t0` to `t1` (seconds) at 10 Hz, without waiting.
pub fn run_demo_until(model: &mut Model, t0: f64, t1: f64) {
    let mut t = t0;
    while t <= t1 {
        for frame in demo_frames(t) {
            apply(model, &frame, t);
        }
        t += 0.1;
    }
}

// ---- live sources, on their own thread --------------------------------------------------

pub type Shared = Arc<Mutex<Model>>;

pub fn spawn_demo(model: Shared, clock: Instant) {
    std::thread::spawn(move || {
        loop {
            let now = clock.elapsed().as_secs_f64();
            let frames = demo_frames(now);
            let mut m = model.lock().unwrap();
            for frame in &frames {
                apply(&mut m, frame, now);
            }
            drop(m);
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

/// Plays a log at `speed` x real time from `from` seconds in, then holds the last state.
pub fn spawn_replay(model: Shared, clock: Instant, path: &Path, from: f64, speed: f64) {
    let path = path.to_owned();
    std::thread::spawn(move || {
        let frames = match candump_frames(&path, from) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("{}: {e}", path.display());
                return;
            }
        };
        let mut count = 0u64;
        for (t_log, frame) in frames {
            let due = t_log / speed;
            let ahead = due - clock.elapsed().as_secs_f64();
            if ahead > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(ahead));
            }
            apply(
                &mut model.lock().unwrap(),
                &frame,
                clock.elapsed().as_secs_f64(),
            );
            count += 1;
        }
        tracing::info!("end of {} after {count} frames", path.display());
    });
}

pub fn spawn_can(model: Shared, clock: Instant, interface: &str) -> std::io::Result<()> {
    use embedded_can::Frame as _;
    use socketcan::{CanSocket, Socket};
    let socket = CanSocket::open(interface)?;
    let interface = interface.to_owned();
    std::thread::spawn(move || {
        loop {
            match socket.read_frame() {
                Ok(socketcan::CanFrame::Data(f)) => {
                    let frame = RawFrame {
                        id: f.id(),
                        data: f.data().to_vec(),
                    };
                    apply(
                        &mut model.lock().unwrap(),
                        &frame,
                        clock.elapsed().as_secs_f64(),
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("{interface}: {e}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candump_lines() {
        let (ts, f) =
            parse_candump_line("(1778835265.757050) can0 00001009#0095022A00000239").unwrap();
        assert_eq!(ts, 1778835265.75705);
        assert_eq!(f.id, Id::Extended(ExtendedId::new(0x1009).unwrap()));
        assert_eq!(f.data, [0x00, 0x95, 0x02, 0x2A, 0x00, 0x00, 0x02, 0x39]);
        let (_, f) = parse_candump_line("(1.0) vcan0 250#2C016AFF0000").unwrap();
        assert_eq!(f.id, Id::Standard(StandardId::new(0x250).unwrap()));
        assert!(parse_candump_line("garbage").is_none());
        assert!(parse_candump_line("(1.0) can0 250#R").is_none());
    }

    #[test]
    fn demo_decodes_to_a_full_view() {
        let mut m = Model::default();
        run_demo_until(&mut m, 0.0, 20.0);
        let v = m.frame(20.0);
        assert!(v.roll_deg.is_some() && v.pitch_deg.is_some());
        assert!(v.des_roll_deg.is_some() && v.des_pitch_deg.is_some());
        assert!(v.height_m.unwrap() > 0.2);
        assert_eq!(v.target_m, Some(0.3));
        assert!(v.fl_aoa_deg.is_some() && v.fr_aoa_deg.is_some());
        assert!(v.rear_aoa_deg.is_some() && v.rear_cmd_aoa_deg.is_some());
        assert!(v.fl_sensor_m.is_some() && v.fr_sensor_m.is_some());
        assert_eq!(v.mode, Some(5));
    }

    #[test]
    fn demo_floats_in_manual_without_a_demand() {
        let mut m = Model::default();
        run_demo_until(&mut m, 0.0, 2.0);
        let v = m.frame(2.0);
        assert_eq!(v.mode, Some(0));
        assert_eq!((v.des_roll_deg, v.des_pitch_deg), (None, None));
        assert!((v.height_m.unwrap() - 0.114).abs() < 1e-3);
    }
}
