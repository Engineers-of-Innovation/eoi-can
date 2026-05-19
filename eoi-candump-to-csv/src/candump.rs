use embedded_can::{ExtendedId, Id, StandardId};

#[derive(Debug, Clone, PartialEq)]
pub struct RawFrame {
    pub timestamp_secs: f64,
    pub id: Id,
    pub data: Vec<u8>,
}

pub fn parse_line(line: &str) -> Option<RawFrame> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let rest = line.strip_prefix('(')?;
    let (ts_str, rest) = rest.split_once(')')?;
    let timestamp_secs: f64 = ts_str.parse().ok()?;

    let rest = rest.trim_start();
    let (_iface, frame) = rest.split_once(' ')?;

    let (id_str, data_str) = frame.split_once('#')?;
    let id = parse_id(id_str)?;
    let data = parse_hex_bytes(data_str)?;

    Some(RawFrame {
        timestamp_secs,
        id,
        data,
    })
}

fn parse_id(s: &str) -> Option<Id> {
    let raw = u32::from_str_radix(s, 16).ok()?;
    if s.len() > 3 {
        ExtendedId::new(raw).map(Id::Extended)
    } else {
        StandardId::new(raw as u16).map(Id::Standard)
    }
}

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(pair, 16).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_id_line() {
        let frame = parse_line("(1778831013.755207) can0 750#A672824089EBA33E").unwrap();
        assert!((frame.timestamp_secs - 1778831013.755207).abs() < 1e-6);
        assert_eq!(frame.id, Id::Standard(StandardId::new(0x750).unwrap()));
        assert_eq!(
            frame.data,
            vec![0xA6, 0x72, 0x82, 0x40, 0x89, 0xEB, 0xA3, 0x3E]
        );
    }

    #[test]
    fn parses_extended_id_line() {
        let frame = parse_line("(1778831013.751851) can0 00000A09#0000000000000000").unwrap();
        assert_eq!(frame.id, Id::Extended(ExtendedId::new(0xA09).unwrap()));
        assert_eq!(frame.data, vec![0u8; 8]);
    }

    #[test]
    fn parses_short_payload() {
        let frame = parse_line("(1778831013.740486) can0 012#028802").unwrap();
        assert_eq!(frame.id, Id::Standard(StandardId::new(0x012).unwrap()));
        assert_eq!(frame.data, vec![0x02, 0x88, 0x02]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_line("").is_none());
        assert!(parse_line("not a candump line").is_none());
        assert!(parse_line("(123.456) can0 750").is_none());
        assert!(parse_line("(123.456) can0 750#oddhex").is_none());
    }
}
