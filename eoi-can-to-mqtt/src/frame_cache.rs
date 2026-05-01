use embedded_can::Id;
use eoi_can_decoder::can_frame::CanFrame;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;

pub struct FrameCache {
    frames: HashMap<Id, (CanFrame, Instant)>,
    dropped_frames: usize,
}

impl FrameCache {
    pub fn new() -> Self {
        Self {
            frames: HashMap::new(),
            dropped_frames: 0,
        }
    }

    pub fn insert(&mut self, frame: CanFrame) {
        let id = frame.id;
        if self.frames.insert(id, (frame, Instant::now())).is_some() {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
        }
    }

    pub fn iter_fresh(&self, max_age: Duration) -> impl Iterator<Item = &CanFrame> {
        let now = Instant::now();
        self.frames
            .values()
            .filter(move |(_, t)| now.duration_since(*t) <= max_age)
            .map(|(f, _)| f)
    }

    pub fn prune(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.frames
            .retain(|_, (_, t)| now.duration_since(*t) <= max_age);
    }

    pub fn take_dropped_frames(&mut self) -> usize {
        std::mem::take(&mut self.dropped_frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_can::{ExtendedId, Id};

    fn frame(id: u32, data: &[u8]) -> CanFrame {
        CanFrame::from_encoded(Id::Extended(ExtendedId::new(id).unwrap()), data)
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_fresh_drops_stale() {
        let mut cache = FrameCache::new();
        cache.insert(frame(0x100, &[1]));

        tokio::time::advance(Duration::from_secs(8)).await;
        cache.insert(frame(0x200, &[2]));

        // At t=8s: both fresh under a 10s TTL.
        assert_eq!(cache.iter_fresh(Duration::from_secs(10)).count(), 2);

        tokio::time::advance(Duration::from_secs(3)).await;
        // At t=11s: 0x100 is 11s old (stale), 0x200 is 3s old (fresh).
        let fresh: Vec<_> = cache.iter_fresh(Duration::from_secs(10)).collect();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, Id::Extended(ExtendedId::new(0x200).unwrap()));

        cache.prune(Duration::from_secs(10));
        assert_eq!(cache.frames.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn replacing_same_id_counts_as_dropped() {
        let mut cache = FrameCache::new();
        cache.insert(frame(0x100, &[1]));
        cache.insert(frame(0x100, &[2]));
        cache.insert(frame(0x100, &[3]));
        assert_eq!(cache.take_dropped_frames(), 2);
        assert_eq!(cache.take_dropped_frames(), 0);
    }
}
