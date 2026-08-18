//! Golden renders: each layout is drawn and hashed, so a change that is meant to
//! move code without moving pixels can be proved to have done so.
//!
//! A failure here is not automatically a bug -- retuning a layout is supposed to
//! change the hash. It means "the pixels moved, confirm you meant that, then
//! update the constant". Run with `--nocapture` to see the hash it got.
//!
//! The top `STAMP_BAND_H` rows are excluded: they carry the build stamp, whose
//! git hash and dirty flag change with the working tree.

use core::convert::Infallible;

use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};

const W: usize = draw_display::DISPLAY_WIDTH as usize;
const H: usize = draw_display::DISPLAY_HEIGHT as usize;
/// Matches `render::STAMP_BAND_H`, which is private.
const STAMP_BAND_H: usize = 8;

struct Canvas {
    /// One entry per pixel, `true` where the layout left the background.
    pixels: Vec<bool>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            pixels: vec![false; W * H],
        }
    }

    /// FNV-1a over one bit per pixel, below the stamp band.
    fn hash(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &on in &self.pixels[STAMP_BAND_H * W..] {
            hash ^= u64::from(on);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// Inked pixels below the stamp band. Reported alongside the hash because it
    /// says *how much* moved -- a hash tells you only that something did.
    fn ink(&self) -> usize {
        self.pixels[STAMP_BAND_H * W..]
            .iter()
            .filter(|on| !**on)
            .count()
    }
}

impl OriginDimensions for Canvas {
    fn size(&self) -> Size {
        Size::new(W as u32, H as u32)
    }
}

impl DrawTarget for Canvas {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        // Clip rather than panic: a layout that overruns the screen is a layout
        // bug for the geometry assertions to catch, not a crash here.
        for Pixel(point, color) in pixels {
            if (0..W as i32).contains(&point.x) && (0..H as i32).contains(&point.y) {
                self.pixels[point.y as usize * W + point.x as usize] = color.is_on();
            }
        }
        Ok(())
    }
}

/// Every value absent, which is what a screen shows on a silent bus: dashes in
/// every cell, no icons raised.
fn stale() -> draw_display::DisplayData {
    draw_display::DisplayData::default()
}

/// Enough fields to put a real figure in every cell the dashboard draws, so the
/// value paths are covered and not just the dashes.
fn populated() -> draw_display::DisplayData {
    let mut data = draw_display::DisplayData::default();
    data.speed_kmh.update(21.65);
    data.gnss_fix.update(draw_display::GnssFix::Fix3D);
    data.battery_state_of_charge.update(87.0);
    data.battery_voltage.update(58.4);
    data.battery_current_in.update(12.5);
    data.battery_current_out_motor.update(-31.2);
    data.battery_current_out_peripherals.update(-1.4);
    data.motor_fet_temperature.update(46.5);
    data.motor_ntc_temperature.update(Some(38.0));
    for (index, temperature) in data.battery_temperatures.iter_mut().enumerate() {
        temperature.update(30 + index as i8);
    }
    for (index, temperature) in data.mppt_temperatures.iter_mut().enumerate() {
        temperature.update(40 + index as i8);
    }
    data
}

#[track_caller]
fn assert_render(
    name: &str,
    draw: fn(&mut Canvas, &draw_display::DisplayData) -> Result<(), Infallible>,
    data: &draw_display::DisplayData,
    expected_hash: u64,
    expected_ink: usize,
) {
    let mut canvas = Canvas::new();
    draw(&mut canvas, data).unwrap();
    let (hash, ink) = (canvas.hash(), canvas.ink());
    println!("{name} hash={hash:#018x} ink={ink}");
    assert_eq!(
        (hash, ink),
        (expected_hash, expected_ink),
        "{name} render changed: got hash {hash:#018x} ink {ink}. \
         If the layout was retuned on purpose, update the expected values."
    );
}

#[test]
fn dashboard_renders_are_unchanged() {
    assert_render(
        "dashboard/stale",
        draw_display::draw_display,
        &stale(),
        0xb908_2cf0_677b_82f2,
        11481,
    );
    assert_render(
        "dashboard/populated",
        draw_display::draw_display,
        &populated(),
        0x0289_f438_6f18_aa2b,
        26100,
    );
}

#[test]
fn foiling_renders_are_unchanged() {
    // Only the stale case: a populated one would need every parameter set up
    // here, which duplicates what the layout's own tests already pin. This
    // catches the labels, hotkeys, headings and column positions moving.
    assert_render(
        "foiling/stale",
        draw_display::draw_foiling,
        &stale(),
        0x0e10_1645_f652_abc9,
        18876,
    );
}
