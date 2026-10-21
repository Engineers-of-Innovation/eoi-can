//! The framebuffer the display boards render into, in the 5.79" panel's own
//! layout: 792 x 272, one bit per pixel, MSB leftmost, bit set = white.
//!
//! Our own rather than epd-waveshare's `Display5in79` because that one can only
//! draw a pixel at a time and exposes no mutable buffer, so a full-screen
//! `clear` went through `set_pixel` 215 424 times on every redraw. Here `clear`
//! is a memset and `fill_solid` writes whole bytes along each row. The layout is
//! the driver's exactly, so the buffer still goes to `Epd5in79` unchanged.
//!
//! Depends on nothing but embedded-graphics, so it can be checked on the host
//! against `Display5in79` bit for bit.

use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;

pub const WIDTH: u32 = 792;
pub const HEIGHT: u32 = 272;
/// No padding at the end of a row: `rotate_180` and the byte-wise fills both
/// rely on it.
const _: () = assert!(WIDTH.is_multiple_of(8));
const ROW_BYTES: usize = WIDTH as usize / 8;
/// Bytes in one panel framebuffer.
pub const FRAME_BYTES: usize = ROW_BYTES * HEIGHT as usize;

/// Draws with the embedded-graphics convention the render code uses, `On` =
/// white. (epd-waveshare's own `From<BinaryColor>` maps `On -> Black`.) If the
/// panel turns out inverted on the bench, flip [`fill_byte`].
pub struct Framebuffer {
    buffer: [u8; FRAME_BYTES],
    /// Draw the image turned half a turn, for a panel that hangs upside down.
    /// Remapped on the way in, so the render code keeps top-left coordinates
    /// and what is clocked out to the panel is already the right way up.
    rotate_180: bool,
}

/// The byte a run of eight `color` pixels is stored as.
const fn fill_byte(color: BinaryColor) -> u8 {
    match color {
        BinaryColor::On => 0xFF,
        BinaryColor::Off => 0x00,
    }
}

impl Framebuffer {
    /// All black. `const` so it can live in a `ConstStaticCell` and never be
    /// built on the stack.
    pub const fn new() -> Self {
        Self {
            buffer: [0; FRAME_BYTES],
            rotate_180: false,
        }
    }

    pub fn set_rotate_180(&mut self, rotate_180: bool) {
        self.rotate_180 = rotate_180;
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Panel coordinates of drawing coordinate `point`, or `None` off-screen.
    fn to_panel(&self, point: Point) -> Option<(usize, usize)> {
        if point.x < 0 || point.y < 0 || point.x >= WIDTH as i32 || point.y >= HEIGHT as i32 {
            return None;
        }
        let (x, y) = (point.x as usize, point.y as usize);
        Some(if self.rotate_180 {
            (WIDTH as usize - 1 - x, HEIGHT as usize - 1 - y)
        } else {
            (x, y)
        })
    }

    fn set(&mut self, x: usize, y: usize, color: BinaryColor) {
        let byte = &mut self.buffer[y * ROW_BYTES + x / 8];
        let bit = 0x80 >> (x % 8);
        *byte = (*byte & !bit) | (fill_byte(color) & bit);
    }

    /// Paints panel row `y` from column `x0` up to, not including, `x1`: bit by
    /// bit up to the first byte boundary, whole bytes to the last one, then bit
    /// by bit again.
    fn fill_span(&mut self, y: usize, x0: usize, x1: usize, color: BinaryColor) {
        let whole_start = x0.next_multiple_of(8).min(x1);
        let whole_end = (x1 & !7).max(whole_start);
        for x in (x0..whole_start).chain(whole_end..x1) {
            self.set(x, y, color);
        }
        let row = y * ROW_BYTES;
        self.buffer[row + whole_start / 8..row + whole_end / 8].fill(fill_byte(color));
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for Framebuffer {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if let Some((x, y)) = self.to_panel(point) {
                self.set(x, y, color);
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let area = area.intersection(&self.bounding_box());
        let Some(bottom_right) = area.bottom_right() else {
            return Ok(()); // zero-sized, or entirely off-screen
        };
        // Both corners are on-screen after the intersection. A half turn maps a
        // rectangle onto a rectangle, so its corners still bound it, just swapped.
        let (Some((ax, ay)), Some((bx, by))) =
            (self.to_panel(area.top_left), self.to_panel(bottom_right))
        else {
            return Ok(());
        };
        let (x0, x1) = (ax.min(bx), ax.max(bx) + 1);
        for y in ay.min(by)..=ay.max(by) {
            self.fill_span(y, x0, x1, color);
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.buffer.fill(fill_byte(color));
        Ok(())
    }
}
