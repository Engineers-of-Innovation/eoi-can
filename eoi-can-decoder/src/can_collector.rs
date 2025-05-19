use crate::can_frame::CanFrame;
#[cfg(feature = "defmt")]
use defmt::error;
use embedded_can::Id;

pub struct CanCollector {
    latest_can_frames: heapless::Vec<CanFrame, 100>,
    dropped_frames: usize,
}

impl CanCollector {
    pub fn new() -> Self {
        Self {
            latest_can_frames: heapless::Vec::new(),
            dropped_frames: usize::default(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &CanFrame> {
        self.latest_can_frames.iter()
    }

    pub fn clear(&mut self) {
        self.dropped_frames = usize::default();
        self.latest_can_frames.clear();
    }

    pub fn insert(&mut self, frame: CanFrame) {
        if self.is_can_id_in_set(frame.id) {
            // If the CAN ID is already in the set, remove it first
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            self.latest_can_frames.retain(|f| f.id != frame.id);
            let _ = self.latest_can_frames.push(frame).map_err(|_e| {
                #[cfg(feature = "defmt")]
                error!("Failed to push CAN frame: {:?}", _e)
            });
        } else {
            if self.latest_can_frames.is_full() {
                // If the set is full, remove the oldest frame
                self.dropped_frames = self.dropped_frames.saturating_add(1);
                self.latest_can_frames.remove(0);
            }
            let _ = self.latest_can_frames.push(frame).map_err(|_e| {
                #[cfg(feature = "defmt")]
                error!("Failed to push CAN frame: {:?}", _e)
            });
        }
    }

    pub fn get_dropped_frames(&self) -> usize {
        self.dropped_frames
    }

    fn is_can_id_in_set(&self, can_id: Id) -> bool {
        self.latest_can_frames
            .iter()
            .any(|frame| frame.id == can_id)
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
        assert!(collector.is_can_id_in_set(frame1.id));
        assert!(collector.is_can_id_in_set(frame2.id));
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
