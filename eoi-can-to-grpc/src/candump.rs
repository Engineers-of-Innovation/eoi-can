use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::live_state::{apply_frame, LiveState};

pub fn parse_line(line: &str) -> Option<(f64, u32, Vec<u8>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if !line.starts_with('(') {
        return None;
    }
    let close = line.find(')')?;
    let ts: f64 = line[1..close].trim().parse().ok()?;
    let rest = line[close + 1..].trim();
    let mut parts = rest.split_whitespace();
    let _iface = parts.next()?;
    let id_data = parts.next()?;
    let (id_s, data_s) = id_data.split_once('#')?;
    let id = u32::from_str_radix(id_s, 16).ok()?;
    let data = hex_bytes(data_s)?;
    Some((ts, id, data))
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = from_hex(bytes[i])?;
        let lo = from_hex(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub async fn replay_loop(path: &Path, looping: bool, state: Arc<Mutex<LiveState>>) {
    loop {
        if let Err(e) = replay_once(path, &state).await {
            tracing::error!(error = %e, "replay failed");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if !looping {
            tracing::info!("end of log; holding last state (5s staleness still applies)");
            break;
        }
        tracing::info!("rewinding log");
    }
}

async fn replay_once(path: &Path, state: &Arc<Mutex<LiveState>>) -> std::io::Result<()> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut log_t0: Option<f64> = None;
    let wall_t0 = Instant::now();

    for line in reader.lines() {
        let line = line?;
        let Some((ts, id, data)) = parse_line(&line) else {
            continue;
        };
        let t0 = *log_t0.get_or_insert(ts);
        let offset = (ts - t0).max(0.0);
        let target = wall_t0 + Duration::from_secs_f64(offset);
        let now = Instant::now();
        if target > now {
            tokio::time::sleep_until(tokio::time::Instant::from_std(target)).await;
        }
        apply_frame(&mut state.lock(), id, &data);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_padded_extended_id() {
        let (ts, id, data) =
            parse_line("(1784381617.837874) can0 00000A09#0000000000000000").unwrap();
        assert!((ts - 1784381617.837874).abs() < 1e-9);
        assert_eq!(id, 0xA09);
        assert_eq!(data, vec![0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn parses_standard_id() {
        let (_, id, data) = parse_line("(1.0) can0 100#aabb").unwrap();
        assert_eq!(id, 0x100);
        assert_eq!(data, vec![0xaa, 0xbb]);
    }
}
