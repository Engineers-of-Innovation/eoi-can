use core::fmt::Write;

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_4X6, FONT_6X10},
        MonoTextStyle, MonoTextStyleBuilder,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use eoi_can_decoder::{BatteryState, ChargeState, DischargeState};
use heapless::String;
use u8g2_fonts::{
    fonts,
    types::{FontColor, HorizontalAlignment, VerticalPosition},
    FontRenderer,
};

use crate::{built_info, DisplayData};

pub const DISPLAY_WIDTH: u32 = 792;
pub const DISPLAY_HEIGHT: u32 = 272;

const COL_W: i32 = DISPLAY_WIDTH as i32 / 3;
/// Extra width given to the left column so its widest unit/label block
/// ("motor") does not run into the next column. Taken from the right column,
/// whose values are right-aligned with slack to spare before the screen edge.
const COL_SHIFT: i32 = 24;
const COL_LEFT: i32 = 0;
const COL_MID: i32 = COL_W + COL_SHIFT;
const COL_RIGHT: i32 = 2 * COL_W + COL_SHIFT;

/// Row centers for the 3-row columns (left, middle).
const ROWS_3: [i32; 3] = [45, 136, 227];
/// Row centers for the 4-row column (right).
const ROWS_4: [i32; 4] = [34, 102, 170, 238];

/// Right-align anchor of the big value within a column, leaving room for the
/// stacked unit/label block after it.
const VALUE_RIGHT_3ROW: i32 = 200;
/// Compensated for `COL_SHIFT` so the right column's content stays put while its
/// origin moves right; it would otherwise be pushed against the screen edge.
const VALUE_RIGHT_4ROW: i32 = 190 - COL_SHIFT;
/// Gap between the big value and the stacked unit/label block.
const BLOCK_GAP: i32 = 10;

// Inconsolata bold: monospaced, so digits don't shift width as values change.
// The clock is 8 monospace glyphs wide (colons count full width), so it needs
// a smaller size than the 5-glyph power values to fit the 264px column.
const FONT_BIG: FontRenderer = FontRenderer::new::<fonts::u8g2_font_inb49_mn>();
const FONT_CLOCK: FontRenderer = FontRenderer::new::<fonts::u8g2_font_inb38_mn>();
const FONT_MID: FontRenderer = FontRenderer::new::<fonts::u8g2_font_inb30_mn>();
const FONT_RIGHT: FontRenderer = FontRenderer::new::<fonts::u8g2_font_inb46_mn>();
const FONT_SMALL: FontRenderer = FontRenderer::new::<fonts::u8g2_font_helvB14_tf>();

/// Offset applied to the GNSS (UTC) time for display, in hours.
const TIME_OFFSET_HOURS: u8 = 2;

/// Glyph coverage is fixed at compile time, so only actual display errors can
/// occur here; anything else is a bug in this module.
fn map_font_err<E>(e: u8g2_fonts::Error<E>) -> E {
    match e {
        u8g2_fonts::Error::DisplayError(e) => e,
        _ => panic!("font rendering failed"),
    }
}

/// Draw a big value with an optional small stacked unit/label block after it.
/// With neither unit nor label the value is centered in the column (clock).
#[allow(clippy::too_many_arguments)]
fn draw_metric<D, C>(
    display: &mut D,
    font: &FontRenderer,
    col_x: i32,
    value_right: i32,
    center_y: i32,
    value: &str,
    unit: Option<&str>,
    label: Option<&str>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let color: C = BinaryColor::Off.into();

    if unit.is_none() && label.is_none() {
        font.render_aligned(
            value,
            Point::new(col_x + COL_W / 2, center_y),
            VerticalPosition::Center,
            HorizontalAlignment::Center,
            FontColor::Transparent(color),
            display,
        )
        .map_err(map_font_err)?;
        return Ok(());
    }

    font.render_aligned(
        value,
        Point::new(col_x + value_right, center_y),
        VerticalPosition::Center,
        HorizontalAlignment::Right,
        FontColor::Transparent(color),
        display,
    )
    .map_err(map_font_err)?;

    let block_x = col_x + value_right + BLOCK_GAP;
    match (unit, label) {
        (Some(unit), Some(label)) => {
            FONT_SMALL
                .render_aligned(
                    unit,
                    Point::new(block_x, center_y - 4),
                    VerticalPosition::Baseline,
                    HorizontalAlignment::Left,
                    FontColor::Transparent(color),
                    display,
                )
                .map_err(map_font_err)?;
            FONT_SMALL
                .render_aligned(
                    label,
                    Point::new(block_x, center_y + 15),
                    VerticalPosition::Baseline,
                    HorizontalAlignment::Left,
                    FontColor::Transparent(color),
                    display,
                )
                .map_err(map_font_err)?;
        }
        (Some(single), None) | (None, Some(single)) => {
            FONT_SMALL
                .render_aligned(
                    single,
                    Point::new(block_x, center_y),
                    VerticalPosition::Center,
                    HorizontalAlignment::Left,
                    FontColor::Transparent(color),
                    display,
                )
                .map_err(map_font_err)?;
        }
        (None, None) => unreachable!(),
    }
    Ok(())
}

/// Format a float with the given number of decimals, or dashes when absent.
fn fmt_f32<'a>(
    buf: &'a mut String<16>,
    value: Option<f32>,
    decimals: usize,
    dashes: &'static str,
) -> &'a str {
    match value {
        Some(v) => {
            buf.clear();
            write!(buf, "{v:.decimals$}").unwrap();
            buf.as_str()
        }
        None => dashes,
    }
}

/// Format hours/minutes/seconds, or --:--:-- when absent.
fn fmt_hms(buf: &mut String<16>, hms: Option<(u8, u8, u8)>) -> &str {
    match hms {
        Some((h, m, s)) => {
            buf.clear();
            write!(buf, "{h:02}:{m:02}:{s:02}").unwrap();
            buf.as_str()
        }
        None => "--:--:--",
    }
}

pub fn draw_display<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    display.clear(BinaryColor::On.into())?;
    let mut buf: String<16> = String::new();

    // Left column: battery power in / motor / net. Currents leaving the
    // battery are negative on the bus, so net = in + motor + peripherals
    // and positive net means the battery is charging.
    let voltage = data.battery_voltage.get().copied();
    let current_in = data.battery_current_in.get().copied();
    let current_motor = data.battery_current_out_motor.get().copied();
    let current_peripherals = data.battery_current_out_peripherals.get().copied();

    let power_in = voltage.zip(current_in).map(|(v, i)| v * i);
    let power_motor = voltage.zip(current_motor).map(|(v, i)| v * i);
    let power_net = (|| Some(voltage? * (current_in? + current_motor? + current_peripherals?)))();

    for (row, power, label) in [
        (0, power_in, "in"),
        (1, power_motor, "motor"),
        (2, power_net, "net"),
    ] {
        draw_metric(
            display,
            &FONT_BIG,
            COL_LEFT,
            VALUE_RIGHT_3ROW,
            ROWS_3[row],
            fmt_f32(&mut buf, power, 0, "---"),
            Some("W"),
            Some(label),
        )?;
    }

    // Middle column: time of day, race time (todo), time to empty (todo).
    let time = data
        .time
        .get()
        .map(|t| ((t.hours + TIME_OFFSET_HOURS) % 24, t.minutes, t.seconds));
    draw_metric(
        display,
        &FONT_CLOCK,
        COL_MID,
        VALUE_RIGHT_3ROW,
        ROWS_3[0],
        fmt_hms(&mut buf, time),
        None,
        None,
    )?;
    // TODO: race time needs a data source (no race-start signal exists yet)
    draw_metric(
        display,
        &FONT_MID,
        COL_MID,
        VALUE_RIGHT_3ROW,
        ROWS_3[1],
        fmt_hms(&mut buf, None),
        None,
        Some("RT"),
    )?;
    // TODO: time to empty is not calculated or sent by anything yet
    draw_metric(
        display,
        &FONT_MID,
        COL_MID,
        VALUE_RIGHT_3ROW,
        ROWS_3[2],
        fmt_hms(&mut buf, None),
        None,
        Some("TTE"),
    )?;

    // Right column: state of charge, speed, motor and driver temperatures.
    draw_metric(
        display,
        &FONT_RIGHT,
        COL_RIGHT,
        VALUE_RIGHT_4ROW,
        ROWS_4[0],
        fmt_f32(
            &mut buf,
            data.battery_state_of_charge.get().copied(),
            0,
            "--",
        ),
        Some("%"),
        None,
    )?;
    draw_metric(
        display,
        &FONT_RIGHT,
        COL_RIGHT,
        VALUE_RIGHT_4ROW,
        ROWS_4[1],
        fmt_f32(&mut buf, data.speed_kmh.get().copied(), 1, "--.-"),
        Some("km/h"),
        None,
    )?;
    draw_metric(
        display,
        &FONT_RIGHT,
        COL_RIGHT,
        VALUE_RIGHT_4ROW,
        ROWS_4[2],
        fmt_f32(&mut buf, data.motor_temperature.get().copied(), 0, "--"),
        Some("°C"),
        Some("motor"),
    )?;
    draw_metric(
        display,
        &FONT_RIGHT,
        COL_RIGHT,
        VALUE_RIGHT_4ROW,
        ROWS_4[3],
        fmt_f32(&mut buf, data.motor_fet_temperature.get().copied(), 0, "--"),
        Some("°C"),
        Some("driver"),
    )?;

    draw_error_badge(display, data)?;
    draw_version(display)?;

    Ok(())
}

/// Small inverted badge in the top-left corner when the battery reports an
/// abnormal state or the throttle reports errors. Stale/unknown values do not
/// trigger it (the dashes already show data loss).
fn draw_error_badge<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let battery_abnormal = !matches!(data.battery_state.get(), None | Some(BatteryState::On))
        || !matches!(
            data.battery_charge_state.get(),
            None | Some(ChargeState::FetOn)
        )
        || !matches!(
            data.battery_discharge_state.get(),
            None | Some(DischargeState::On)
        );
    let throttle_error = data.throttle_errors.get().is_some_and(|e| e.has_error());

    if !battery_abnormal && !throttle_error {
        return Ok(());
    }

    let style: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On.into())
        .background_color(BinaryColor::Off.into())
        .build();

    let mut badge: String<16> = String::new();
    if battery_abnormal {
        badge.push_str(" BAT! ").unwrap();
    }
    if throttle_error {
        badge.push_str(" THR! ").unwrap();
    }
    Text::new(badge.as_str(), Point::new(4, 10), style).draw(display)?;
    Ok(())
}

fn draw_version<D, C>(display: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let style: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_4X6)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();

    let mut version: String<64> = String::new();
    write!(
        &mut version,
        "Version: {}, Git: {:.8}{}",
        built_info::PKG_VERSION,
        built_info::GIT_COMMIT_HASH.unwrap_or("unknown"),
        if built_info::GIT_DIRTY.unwrap_or(false) {
            "-dirty"
        } else {
            ""
        }
    )
    .unwrap();

    Text::with_alignment(
        version.as_str(),
        Point::new(DISPLAY_WIDTH as i32 - 4, DISPLAY_HEIGHT as i32 - 4),
        style,
        Alignment::Right,
    )
    .draw(display)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_f32_formats_and_dashes() {
        let mut buf: String<16> = String::new();
        assert_eq!(fmt_f32(&mut buf, Some(1234.4), 0, "---"), "1234");
        assert_eq!(fmt_f32(&mut buf, Some(-950.6), 0, "---"), "-951");
        assert_eq!(fmt_f32(&mut buf, Some(12.34), 1, "--.-"), "12.3");
        assert_eq!(fmt_f32(&mut buf, None, 0, "---"), "---");
    }

    #[test]
    fn fmt_hms_formats_and_dashes() {
        let mut buf: String<16> = String::new();
        assert_eq!(fmt_hms(&mut buf, Some((23, 55, 1))), "23:55:01");
        assert_eq!(fmt_hms(&mut buf, None), "--:--:--");
    }

    /// The unit/label block is the widest thing on the right of each column, so
    /// it is what runs into the next one. Guards `COL_SHIFT` against font or
    /// label changes: "motor" used to end exactly on the old column boundary.
    #[test]
    fn label_blocks_clear_the_next_column() {
        const MIN_CLEARANCE: i32 = 8;

        for (col_x, value_right, widest_label, next_edge) in [
            (COL_LEFT, VALUE_RIGHT_3ROW, "motor", COL_MID),
            (COL_MID, VALUE_RIGHT_3ROW, "TTE", COL_RIGHT),
            (COL_RIGHT, VALUE_RIGHT_4ROW, "km/h", DISPLAY_WIDTH as i32),
        ] {
            let width = FONT_SMALL
                .get_rendered_dimensions(widest_label, Point::zero(), VerticalPosition::Center)
                .unwrap()
                .advance
                .x;
            let block_right = col_x + value_right + BLOCK_GAP + width;
            assert!(
                block_right + MIN_CLEARANCE <= next_edge,
                "{widest_label:?} ends at {block_right}, too close to {next_edge}"
            );
        }
    }
}
