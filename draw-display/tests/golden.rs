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

    /// Write the whole canvas, stamp band included, as a binary PGM.
    ///
    /// Set `SCREENSHOT_DIR` and every golden case drops a file there, which is how
    /// the layouts get looked at without a panel or a CAN bus. PGM because it
    /// needs no encoder: the goldens must stay dependency-free, and anything that
    /// reads images reads this.
    fn write_pgm(&self, path: &std::path::Path) {
        let mut out = format!("P5\n{W} {H}\n255\n").into_bytes();
        out.extend(self.pixels.iter().map(|&on| if on { 0xFF } else { 0x00 }));
        std::fs::write(path, out).expect("screenshot written");
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
    // Course over ground: 127 deg draws as "127°" under a "Heading / SE" label.
    data.heading_deg.update(127.0);
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
    // UTC, as it arrives on `0x204`: an August date, so the clock draws 14:32:11
    // and the summer-time offset is part of what this render pins.
    data.time.update(draw_display::GnssDateTime {
        year: 2026,
        month: 8,
        day: 28,
        hours: 12,
        minutes: 32,
        seconds: 11,
    });
    // 39:45 of endurance, the estimate `update_endurance` produces from the
    // currents above.
    data.battery_endurance
        .update(draw_display::Endurance::ToEmpty(2385));

    // Fourteen cells around 3.9 V with one weak one, so the information screen's
    // lowest-cell and spread both have something to say.
    for (index, cell) in data.battery_cell_voltages.iter_mut().enumerate() {
        cell.update(3.948 + (index % 3) as f32 * 0.011);
    }
    data.battery_cell_voltages[6].update(3.902);
    data.battery_current_pack.update(-20.1);

    // The eleven MPPTs `LAYOUT` places, by ID strap so they land in CAN address
    // order: R0-R7 then F1, F4, F7. Input is the panel at its own voltage, output
    // is the pack bus, and the two powers differ by the converter's losses.
    for (strap, input_voltage, input_current) in [
        (0u8, 34.8, 5.9),
        (1, 35.2, 6.1),
        (2, 34.1, 5.4),
        (3, 35.9, 6.3),
        (4, 33.7, 4.8),
        (5, 35.1, 6.0),
        (6, 34.4, 5.6),
        (7, 35.5, 6.2),
        (9, 32.9, 4.1),
        (12, 33.4, 4.6),
        (15, 34.0, 5.1),
    ] {
        let input_power = input_voltage * input_current;
        let output_voltage = 58.4;
        data.mppt_power[strap as usize].update(draw_display::MpptPowerFlow {
            input_voltage,
            input_current,
            output_voltage,
            // 96 % efficient, which is what makes the two power columns worth
            // having side by side.
            output_current: input_power * 0.96 / output_voltage,
        });
        // The heat sink leads the board, as it does on a working unit. Only the
        // straps above report at all: a unit that is not plugged in says nothing,
        // which is what gives the information screen a table as long as the bus.
        data.mppt_heat[strap as usize].update(draw_display::MpptHeat {
            board: 38 + (strap as i8 % 5),
            heat_sink: 44 + (strap as i8 % 7),
        });
    }

    data.motor_rpm.update(21_400); // electrical; 2140 rpm at the shaft
    data.motor_duty_cycle.update(62.4);
    data.motor_current.update(74.8);
    data.motor_battery_voltage.update(57.9);
    data.motor_battery_current.update(46.3);
    // 24.8 % to starboard, from a controller whose calibration is valid.
    data.steering_position.update(Some(24.8));
    data.throttle_value.update(58.2);
    data.height_sensor_front_left.update(412);
    data.height_sensor_front_right.update(438);
    data.water_temperature_in.update(Some(21.4));
    data.water_temperature_out.update(Some(28.9));
    data.water_flow_in.update(1880);
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
    if let Ok(dir) = std::env::var("SCREENSHOT_DIR") {
        canvas
            .write_pgm(&std::path::Path::new(&dir).join(format!("{}.pgm", name.replace('/', "-"))));
    }
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
        0xb2d8_63c0_e751_5835,
        11210,
    );
    assert_render(
        "dashboard/populated",
        draw_display::draw_display,
        &populated(),
        0x1408_e4c7_76c0_0ca8,
        28997,
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
        0x4d6c_bd40_8314_c586,
        15797,
    );
}

/// The populated case, which is what pins the MPPT table: eleven units in CAN
/// address order, both sides of each converter, and both its temperatures.
#[test]
fn information_renders_are_unchanged_populated() {
    assert_render(
        "information/populated",
        draw_display::draw_information,
        &populated(),
        0x28f4_d8d6_9491_1d45,
        22008,
    );
}

/// The silent-bus case: every heading and unit up, every value dashed, and no
/// MPPT rows at all -- the table is as long as the bus is, and the bus is empty.
#[test]
fn information_renders_are_unchanged() {
    assert_render(
        "information/stale",
        draw_display::draw_information,
        &stale(),
        0x9bd0_412b_6f69_c9b0,
        7873,
    );
}
