use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use epd_waveshare::epd7in5_v2::Display7in5;
use epd_waveshare::prelude::Color;

/// Wraps [`Display7in5`] so the shared `draw_display` code (which uses the
/// standard embedded-graphics convention, `On` = white) renders correctly on
/// the panel.
///
/// epd-waveshare's built-in `From<BinaryColor> for Color` maps `On -> Black`,
/// the opposite of the simulator/framebuffer. epd7in5_v2 PR #258 fixed the
/// panel-level inversion (`Color::White` now renders white), which exposed that
/// mismatch: `clear(BinaryColor::On)` became a black background. This wrapper
/// maps `On -> White` / `Off -> Black` so all three outputs (firmware,
/// simulator, framebuffer) match.
pub struct EpdDisplay(pub Display7in5);

impl EpdDisplay {
    pub fn new() -> Self {
        Self(Display7in5::default())
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
        self.0
            .draw_iter(pixels.into_iter().map(|Pixel(point, color)| {
                let color = match color {
                    BinaryColor::On => Color::White,
                    BinaryColor::Off => Color::Black,
                };
                Pixel(point, color)
            }))
    }
}
