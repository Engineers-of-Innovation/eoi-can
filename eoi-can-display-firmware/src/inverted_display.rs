use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use epd_waveshare::epd5in79::Display5in79;
use epd_waveshare::prelude::Color;

/// Wraps [`Display5in79`] so the shared `draw_display` code (which uses the
/// standard embedded-graphics convention, `On` = white) renders correctly on
/// the panel.
///
/// epd-waveshare's built-in `From<BinaryColor> for Color` maps `On -> Black`,
/// the opposite of the simulator/framebuffer. The epd5in79 driver writes the
/// buffer raw with `Color::White` = bit 1 = white on the panel, so without
/// this wrapper `clear(BinaryColor::On)` would paint a black background. It
/// maps `On -> White` / `Off -> Black` so all three outputs (firmware,
/// simulator, framebuffer) match. If the panel turns out inverted on the
/// bench, flip the mapping here — single point of change.
pub struct EpdDisplay(pub Display5in79);

impl EpdDisplay {
    pub fn new() -> Self {
        Self(Display5in79::default())
    }

    pub fn buffer(&self) -> &[u8] {
        self.0.buffer()
    }
}

impl OriginDimensions for EpdDisplay {
    fn size(&self) -> Size {
        self.0.size()
    }
}

impl DrawTarget for EpdDisplay {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        self.0.draw_iter(pixels.into_iter().map(|Pixel(point, color)| {
            let color = match color {
                BinaryColor::On => Color::White,
                BinaryColor::Off => Color::Black,
            };
            Pixel(point, color)
        }))
    }
}
