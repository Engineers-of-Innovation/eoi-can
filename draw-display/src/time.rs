pub use embassy_time::Duration;

#[cfg(not(feature = "tokio"))]
#[expect(clippy::disallowed_types)]
pub type Instant = embassy_time::Instant;

#[cfg(feature = "tokio")]
#[derive(Debug)]
pub struct Instant(tokio::time::Instant);

#[cfg(feature = "tokio")]
impl Instant {
    pub fn now() -> Self {
        Self(tokio::time::Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed().try_into().unwrap()
    }
}
