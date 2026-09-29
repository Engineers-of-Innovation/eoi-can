use crate::can_frame::CanFrame;
use embedded_can::Id;
use heapless::FnvIndexMap;

/// CAN IDs that carry more than one distinct reading, discriminated by their
/// first data byte.
///
/// `0x261` is `foil_tune.lua`'s parameter read-back: fifty parameters share the
/// one ID and differ only by the index in byte 0. Keyed by ID alone they would
/// overwrite each other and all but the last of a dump would be counted as
/// dropped -- the display would learn one gain per redraw, which at a second per
/// refresh is most of a minute to fill a screen.
///
/// `0x263` is the same shape for the nine configuration slots, keyed by the slot
/// number in byte 0: eight of the nine would otherwise be dropped from every
/// broadcast, and the column would show one slot's label at a time.
const MULTIPLEXED_IDS: [u32; 2] = [0x261, 0x263];

/// Key for one distinct reading: the raw ID, plus the sub-address for the IDs that
/// carry several.
fn key_for(frame: &CanFrame) -> u64 {
    let raw = match frame.id {
        Id::Standard(id) => u32::from(id.as_raw()),
        Id::Extended(id) => id.as_raw(),
    };
    let sub = if MULTIPLEXED_IDS.contains(&raw) {
        // Absent on a truncated frame, which then keys as sub-address 0 and is
        // rejected by the decoder later anyway.
        u64::from(frame.data.first().copied().unwrap_or(0))
    } else {
        0
    };
    // Extended IDs are 29 bits, so shifting by 8 cannot collide with the
    // sub-address.
    (u64::from(raw) << 8) | sub
}

pub struct CanCollector {
    /// 256 rather than 128: the documented map already runs to ~68 IDs and a
    /// single `0x261` dump adds fifty more, which would not fit.
    latest_can_frames: FnvIndexMap<u64, CanFrame, 256>,
    dropped_frames: usize,
}

impl CanCollector {
    pub const fn new() -> Self {
        Self {
            latest_can_frames: FnvIndexMap::new(),
            dropped_frames: 0,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &CanFrame> {
        self.latest_can_frames.values()
    }

    pub fn clear(&mut self) {
        self.dropped_frames = usize::default();
        self.latest_can_frames.clear();
    }

    pub fn insert(&mut self, frame: CanFrame) {
        let key = key_for(&frame);
        match self.latest_can_frames.insert(key, frame) {
            Ok(None) => {}
            Ok(Some(_)) => {
                self.dropped_frames = self.dropped_frames.saturating_add(1);
            }
            Err(_) => self.dropped_frames = self.dropped_frames.saturating_add(1),
        }
    }

    pub fn get_dropped_frames(&self) -> usize {
        self.dropped_frames
    }
}

impl Default for CanCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use embedded_can::{ExtendedId, Id};

    use super::*;
    use crate::can_frame::CanFrame;
    use assert2::assert;

    #[test]
    fn test_can_collector() {
        let mut collector = CanCollector::new();
        let frame1 = CanFrame::from_encoded(
            Id::Extended(ExtendedId::new(0x12345).unwrap()),
            &[0x01, 0x02, 0x03],
        );
        let frame1_mirrored = CanFrame::from_encoded(
            Id::Extended(ExtendedId::new(0x12345).unwrap()),
            &[0x03, 0x02, 0x01],
        );
        let frame2 = CanFrame::from_encoded(
            Id::Extended(ExtendedId::new(0x12346).unwrap()),
            &[0x01, 0x02, 0x03],
        );
        let frame2_mirrored = CanFrame::from_encoded(
            Id::Extended(ExtendedId::new(0x12346).unwrap()),
            &[0x01, 0x02, 0x03],
        );

        assert!(collector.iter().count() == 0);
        assert!(collector.get_dropped_frames() == 0);

        collector.insert(frame1.clone());
        collector.insert(frame2.clone());

        assert!(collector.iter().count() == 2);
        assert!(collector.get_dropped_frames() == 0);

        collector.clear();
        assert!(collector.iter().count() == 0);

        collector.insert(frame1.clone());
        assert!(collector.iter().count() == 1);
        collector.insert(frame1.clone()); // Inserting the same frame again should not change the count
        assert!(collector.iter().count() == 1);
        collector.insert(frame2.clone());
        assert!(collector.iter().count() == 2);
        collector.insert(frame2.clone()); // Inserting the same frame again should not change the count
        assert!(collector.iter().count() == 2);
        assert!(collector.get_dropped_frames() == 2);

        collector.clear();
        assert!(collector.iter().count() == 0);
        collector.insert(frame1.clone());
        assert!(collector.iter().next() == Some(&frame1));
        collector.insert(frame1_mirrored.clone());
        assert!(collector.iter().next() == Some(&frame1_mirrored));
        assert!(collector.iter().count() == 1); // Should still be 1, as frame1_mirror replaces frame1
        collector.insert(frame2.clone());
        assert!(collector.iter().count() == 2);
        assert!(collector.iter().next() == Some(&frame1_mirrored));
        assert!(collector.iter().nth(1) == Some(&frame2));
        collector.insert(frame2_mirrored.clone());
        assert!(collector.iter().count() == 2); // Should still be 2, as frame2_mirror replaces frame2
        assert!(collector.iter().next() == Some(&frame1_mirrored));
        assert!(collector.iter().nth(1) == Some(&frame2_mirrored));
        assert!(collector.get_dropped_frames() == 2);
    }
}

#[cfg(test)]
mod multiplexed_tests {
    use super::*;
    use crate::can_frame::CanFrame;
    use embedded_can::StandardId;

    fn param(index: u8) -> CanFrame {
        CanFrame::from_encoded(
            Id::Standard(StandardId::new(0x261).unwrap()),
            &[index, 0, 0, 0, 0, 0],
        )
    }

    /// A whole `foil_tune.lua` dump has to survive one drain. Keyed by ID alone,
    /// fifty parameters sharing `0x261` would leave one frame and forty-nine
    /// counted as dropped.
    #[test]
    fn a_parameter_dump_is_not_coalesced() {
        let mut collector = CanCollector::new();
        let indices: [u8; 50] = core::array::from_fn(|i| i as u8 + 1);
        for index in indices {
            collector.insert(param(index));
        }
        assert_eq!(collector.iter().count(), 50);
        assert_eq!(collector.get_dropped_frames(), 0);
    }

    /// Re-sending the same parameter still replaces, so a stale reading cannot
    /// linger behind a fresh one.
    #[test]
    fn the_same_parameter_still_replaces() {
        let mut collector = CanCollector::new();
        collector.insert(param(16));
        collector.insert(param(16));
        assert_eq!(collector.iter().count(), 1);
        assert_eq!(collector.get_dropped_frames(), 1);
    }

    /// An ID that is not multiplexed keeps the old behaviour: newest wins, and the
    /// map does not grow an entry per payload.
    #[test]
    fn an_ordinary_id_is_still_latest_wins() {
        let mut collector = CanCollector::new();
        for value in 0..10u8 {
            collector.insert(CanFrame::from_encoded(
                Id::Standard(StandardId::new(0x250).unwrap()),
                &[value, 0, 0, 0],
            ));
        }
        assert_eq!(collector.iter().count(), 1);
        assert_eq!(collector.get_dropped_frames(), 9);
    }
}
