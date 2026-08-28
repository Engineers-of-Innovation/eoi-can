//! The helm dashboard: speed in the centre, power on the left, temperatures on
//! the right, and three times across the bottom.
//!
//! Geometry only -- the `Cell` model, the fonts and every widget come from
//! [`super`]. What lives here is where things go, and the compile-time
//! assertions that keep them from colliding as the constants are retuned.

use core::fmt::Write;

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Alignment,
};
use eoi_can_decoder::{BatteryState, ChargeState, DischargeState};
use heapless::String;
use u8g2_fonts::types::HorizontalAlignment;

use super::*;
use crate::{built_info, DisplayData, GnssFix, MpptId, Side};

/// Height of the full-width bottom row holding the three times. Setting it also
/// sets where the band above ends, and so how far down the icons, temperatures and
/// in/out rows sit.
const ROW_BOTTOM_H: i32 = 58;

/// Tallest thing in a time block: the two-line label, not the digits.
const TIME_BLOCK_H: i32 = 2 * SMALL_CAP_H + STACK_LABEL_GAP;
/// Air left under the times.
const TIME_BOTTOM_GAP: i32 = 4;
/// The times hang off the bottom of the screen rather than sitting centred in their
/// row, which puts them as low as they can go and leaves the band above roomier.
const TIME_CENTER_Y: i32 = SCREEN.bottom() - TIME_BOTTOM_GAP - TIME_BLOCK_H / 2;
const TIME_TOP_Y: i32 = TIME_CENTER_Y - TIME_BLOCK_H / 2;

/// Everything above the bottom row: the three metric columns. The boundary is not
/// drawn -- the bands read as separate on spacing alone -- but it still positions
/// what sits against it.
const BAND_TOP: Cell = Cell {
    h: SCREEN.h - ROW_BOTTOM_H,
    ..SCREEN
};
/// 35 / 30 / 35 split: the centre column is narrower, the outer two carry the
/// wider power and temperature values.
const TOP_COLS: [Cell; 3] = BAND_TOP.cols([35, 30, 35]);
const COL_LEFT: Cell = TOP_COLS[0];
const COL_MID: Cell = TOP_COLS[1];
const COL_RIGHT: Cell = TOP_COLS[2];

/// Keeps column contents off each other and off the screen edges. There are no
/// vertical rules any more, so this is the only thing separating the columns.
const COL_PAD_X: i32 = 20;
/// Vertical inset of the outer columns. Small on purpose -- the panel is white
/// past the active area, so tight margins read fine. The headline takes its top
/// from `STAMP_BAND_H` instead, to clear the stamp line above it.
const COL_PAD_Y: i32 = 4;

const LEFT_INNER: Cell = COL_LEFT.inset(COL_PAD_X, COL_PAD_Y);
const RIGHT_INNER: Cell = COL_RIGHT.inset(COL_PAD_X, COL_PAD_Y);

/// Both outer columns open with a headline value over a small label -- net power
/// on the left, state of charge on the right. They share one block so the two big
/// figures sit on the same line.
const HEADLINE_H: i32 = NET_DIGIT_H + NET_STACK_GAP + SMALL_CAP_H;
const HEADLINE_BLOCK: Cell = Cell {
    x: 0,
    y: BAND_TOP.y + STAMP_BAND_H,
    w: 0,
    h: HEADLINE_H,
};
const HEADLINE_STACK: [i32; 2] =
    HEADLINE_BLOCK.stack_centers([NET_DIGIT_H, SMALL_CAP_H], NET_STACK_GAP);
const HEADLINE_VALUE_Y: i32 = HEADLINE_STACK[0];
const HEADLINE_LABEL_Y: i32 = HEADLINE_STACK[1];

/// Everything below the headline in the left column.
const LEFT_BODY: Cell = Cell {
    y: HEADLINE_BLOCK.bottom(),
    h: LEFT_INNER.bottom() - HEADLINE_BLOCK.bottom(),
    ..LEFT_INNER
};

/// State of charge shares the net power font, with a percent sign after it at the
/// same size as the speed's tenth. The pair is right-aligned on the column, so the
/// sign lands on the same edge as the temperatures below it.
const SOC_PCT_X: i32 = RIGHT_INNER.right() - BIG_PCT_W;
const SOC_VALUE_RIGHT: i32 = SOC_PCT_X - BLOCK_GAP;
/// Left edge of the widest state of charge, "100". The nearest thing in the right
/// column to the speed, and so what actually bounds it -- the column inset is a
/// long way left of any ink.
const SOC_VALUE_LEFT: i32 = SOC_VALUE_RIGHT - 3 * NET_DIGIT_W;
/// The sign sits on the same bottom edge as the digits, as the speed's tenth does.
const SOC_PCT_CENTER_Y: i32 = HEADLINE_VALUE_Y + NET_DIGIT_H / 2 - BIG_DIGIT_H / 2;

/// Width reserved for a temperature's digits, to the left of its degree sign:
/// three digits, so the sign holds still whether the value is 8 or 108. The draw
/// code right-aligns from the cell edge instead, so this only records the space
/// the grid must leave -- `widest_values_fit_their_cells` enforces it.
#[allow(dead_code)]
const TEMP_FIELD_W: i32 = 3 * MID_DIGIT_W;
/// Height of one temperature: value over label.
const TEMP_BLOCK_H: i32 = MID_DIGIT_H + LABEL_GAP + SMALL_CAP_H;
/// Gap between the two rows of the grid.
const TEMP_ROW_GAP: i32 = 20;
/// Air between the grid and the times under it. Measured against the times for the
/// same reason the icons are: the band boundary is not drawn.
const TEMP_BOTTOM_GAP: i32 = 18;

/// Four temperatures in a 2x2 grid: motor and driver, then the hottest MPPT and
/// the hottest battery thermistor. Pinned to the bottom of the column rather than
/// centred in the space below the headline, so it sits at the bottom of the band.
const TEMP_GRID: Cell = Cell {
    x: RIGHT_INNER.x,
    y: TIME_TOP_Y - TEMP_BOTTOM_GAP - (2 * TEMP_BLOCK_H + TEMP_ROW_GAP),
    w: RIGHT_INNER.w,
    h: 2 * TEMP_BLOCK_H + TEMP_ROW_GAP,
};
const TEMP_ROWS: [Cell; 2] = TEMP_GRID.rows();

// The grid must clear the headline's label line above it.
const _: () = assert!(TEMP_GRID.y >= HEADLINE_LABEL_Y + SMALL_CAP_H / 2);

/// Gap between the net power value and its label line.
const NET_STACK_GAP: i32 = 8;

/// The in/out values are right-aligned against their labels, which start here.
const POWER_LEFT_X: i32 = LEFT_INNER.x;

/// The widest net power: a minus and four digits.
const NET_FIELD_W: i32 = NET_MINUS_W + 4 * NET_DIGIT_W;
/// The net power figure is right-justified and grows leftwards, so its last digit
/// holds still. The anchor puts the widest value flush against the screen edge --
/// the panel is white past the active area, so there is nothing to gain by
/// padding it off.
const NET_VALUE_RIGHT: i32 = COL_LEFT.x + NET_FIELD_W;

/// Advance of "Net Power" in `FONT_SMALL`, pinned by
/// `font_metrics_match_the_layout`.
const NET_LABEL_W: i32 = 92;
/// How far left of the value's left edge the label is pulled. Nothing sits at the
/// screen's left edge, so the label may reach outside the column padding.
const NET_LABEL_SHIFT: i32 = 16;
/// "Net Power" is right-justified, so its right edge holds still if the text
/// changes rather than the label growing rightwards under the value.
const NET_LABEL_RIGHT: i32 = POWER_LEFT_X + NET_LABEL_W - NET_LABEL_SHIFT;

/// The net power unit sits a digit in from the column edge rather than hard
/// against it.
const NET_UNIT_RIGHT: i32 = LEFT_INNER.right() - NET_DIGIT_W;

// Const arithmetic, so these are compile-time checks: a headline value must not
// overlap its label line, and the label must stay clear of the body below.
const _: () = assert!(HEADLINE_VALUE_Y + NET_DIGIT_H / 2 < HEADLINE_LABEL_Y - SMALL_CAP_H / 2);
const _: () = assert!(HEADLINE_LABEL_Y + SMALL_CAP_H / 2 <= LEFT_BODY.y);

/// Height of one in/out row: the value, or its two-line label, whichever is taller.
const IN_OUT_BLOCK_H: i32 = if MID_DIGIT_H > 2 * SMALL_CAP_H + STACK_LABEL_GAP {
    MID_DIGIT_H
} else {
    2 * SMALL_CAP_H + STACK_LABEL_GAP
};
/// Value centres of the temperature grid's two rows.
const TEMP_VALUE_CENTERS: [i32; 2] = [
    TEMP_ROWS[0].stack_centers([MID_DIGIT_H, SMALL_CAP_H], LABEL_GAP)[0],
    TEMP_ROWS[1].stack_centers([MID_DIGIT_H, SMALL_CAP_H], LABEL_GAP)[0],
];

/// How far a label sits under its value, taken from the temperature grid so the
/// in/out rows match it exactly rather than by eye.
const TEMP_LABEL_OFFSET: i32 = TEMP_ROWS[0].stack_centers([MID_DIGIT_H, SMALL_CAP_H], LABEL_GAP)[1]
    - TEMP_ROWS[0].stack_centers([MID_DIGIT_H, SMALL_CAP_H], LABEL_GAP)[0];

/// The in/out pair takes its y from the temperatures opposite, so the two columns
/// line up on both lines and stay lined up when the grid moves.
const IN_OUT_CENTERS: [i32; 2] = TEMP_VALUE_CENTERS;

// The pair has to sit clear of the headline above and inside the body.
const _: () = assert!(IN_OUT_CENTERS[0] - IN_OUT_BLOCK_H / 2 >= LEFT_BODY.y);
const _: () = assert!(IN_OUT_CENTERS[1] + IN_OUT_BLOCK_H / 2 <= LEFT_BODY.bottom());

/// Width reserved for an in/out value: a minus and four digits, the widest these
/// can reach. The values are left-aligned and shorter ones simply leave a gap, so
/// that both stacked labels sit at the same x instead of tracking the digits.
const POWER_FIELD_W: i32 = MID_MINUS_W + 4 * MID_DIGIT_W;
/// Right edge of an in/out reading: value, unit, and the label under them all end
/// here, the same shape as a temperature.
const IN_OUT_RIGHT: i32 = POWER_LEFT_X + POWER_FIELD_W + VALUE_UNIT_GAP + SMALL_W_W;
/// Two-line labels are gone: the unit drops beside the digits and the name goes
/// underneath, matching the temperature grid opposite.
const POWER_IN_LABEL: &str = "Power In";
const POWER_OUT_LABEL: &str = "Power Out";

/// Gap between the speed and the fix/unit line under it.
const SPEED_STACK_GAP: i32 = 12;

/// Visible air between the speed's digits and the dot, and between the dot and the
/// tenth. Measured ink-to-ink: each glyph carries side bearings inside its cell or
/// advance, so positioning on those alone leaves a gap roughly twice this wide.
const SPEED_DOT_GAP: i32 = 8;
/// Two whole digits, the dot, and the tenth.
const SPEED_BLOCK_W: i32 = 2 * SPEED_DIGIT_W + SPEED_DOT_W + BIG_DIGIT_W + 2 * SPEED_DOT_GAP;
/// Nudges the whole speed right of the column centre -- digits, dot, tenth and the
/// fix/unit line under them, since all of those derive from the block.
const SPEED_SHIFT: i32 = 6;
const SPEED_BLOCK_X: i32 = COL_MID.center_x() - SPEED_BLOCK_W / 2 + SPEED_SHIFT;

/// Nudges the whole numbers alone, independently of the dot and tenth beside them.
const SPEED_INT_SHIFT: i32 = 2;
/// Right edge of the whole-number cells: they grow leftwards from here, so the dot
/// never moves as the speed crosses 10 km/h.
const SPEED_INT_RIGHT: i32 = SPEED_BLOCK_X + 2 * SPEED_DIGIT_W + SPEED_INT_SHIFT;
/// The dot and the tenth hang off the digits rather than off the block, so the gap
/// between them is set by `SPEED_DOT_GAP` alone.
const SPEED_DOT_X: i32 = SPEED_INT_RIGHT - SPEED_DIGIT_BEARING + SPEED_DOT_GAP - BIG_DOT_BEARING;
const SPEED_DEC_X: i32 =
    SPEED_DOT_X + BIG_DOT_BEARING + SPEED_DOT_W + SPEED_DOT_GAP - BIG_DIGIT_BEARING;

// The speed is wider than the centre column and that is fine -- no rule is drawn
// there. What it must not do is reach its neighbours' ink: the net power's right
// edge on one side, the state of charge's left on the other.
const _: () = assert!(SPEED_BLOCK_X >= NET_VALUE_RIGHT + 8);
const _: () = assert!(SPEED_BLOCK_X + SPEED_BLOCK_W <= SOC_VALUE_LEFT - 8);
// Ink must not touch ink: the dot starts after the digits end, and the tenth after
// the dot. Compared on ink edges, not cell edges -- the bearings are the point.
const _: () = assert!(SPEED_INT_RIGHT - SPEED_DIGIT_BEARING < SPEED_DOT_X + BIG_DOT_BEARING);
const _: () =
    assert!(SPEED_DOT_X + BIG_DOT_BEARING + SPEED_DOT_W <= SPEED_DEC_X + BIG_DIGIT_BEARING);

/// Centre column: the speed, with the fix state and unit on a line beneath.
const MID_STACK: [i32; 2] = COL_MID
    .inset(0, COL_PAD_Y)
    .stack_centers([SPEED_DIGIT_H, SMALL_CAP_H], SPEED_STACK_GAP);
/// Lifts the whole speed block -- digits and the fix/unit line under them -- to
/// leave room beneath for icons.
const SPEED_LIFT: i32 = 16;
const SPEED_CENTER_Y: i32 = MID_STACK[0] - SPEED_LIFT;
const SPEED_INFO_Y: i32 = MID_STACK[1] - SPEED_LIFT;

/// The tenth is drawn at the right column's size, sitting on the same bottom edge
/// as the whole numbers.
const SPEED_DEC_CENTER_Y: i32 = SPEED_CENTER_Y + SPEED_DIGIT_H / 2 - BIG_DIGIT_H / 2;

/// Icon strip under the speed. 48px squares with a gap between; the four together
/// are slightly wider than the centre column, which is fine -- nothing else sits at
/// that height, and they read best centred on the speed above them.
const ICON_SIZE: i32 = 48;
const ICON_GAP: i32 = 16;
const ICON_COUNT: i32 = 4;
const ICON_STRIP_W: i32 = ICON_COUNT * ICON_SIZE + (ICON_COUNT - 1) * ICON_GAP;
/// Nudges the strip right of the column centre.
const ICON_SHIFT: i32 = 4;
const ICON_STRIP_X: i32 = COL_MID.center_x() - ICON_STRIP_W / 2 + ICON_SHIFT;
/// Air between the icon strip and the times under it. Measured against the times
/// rather than the band boundary, which is not drawn and so is not what the eye
/// reads the strip as sitting above.
const ICON_BOTTOM_GAP: i32 = 16;
/// `SPEED_LIFT` is what makes room for the strip.
const ICON_STRIP_Y: i32 = TIME_TOP_Y - ICON_BOTTOM_GAP - ICON_SIZE;
/// Bottom of the speed's fix/unit line -- the top of the space below the speed.
const ICON_BAND_TOP: i32 = SPEED_INFO_Y + SMALL_CAP_H / 2;

// The strip has to fit under the fix/unit line without crossing into the times.
const _: () = assert!(ICON_STRIP_Y >= ICON_BAND_TOP);
const _: () = assert!(ICON_STRIP_Y + ICON_SIZE < TIME_TOP_Y);

// The stamp line must stay clear of the headline digits under it -- that is what
// `STAMP_BAND_H` buys.
const _: () = assert!(VERSION_BASELINE_Y < HEADLINE_VALUE_Y - NET_DIGIT_H / 2);

/// The fix state starts under the last whole digit -- the one always on screen,
/// whether the speed reads 9 or 29. Anchoring to the first digit instead would move
/// the label every time the speed crossed 10 km/h.
const SPEED_FIX_X: i32 = SPEED_INT_RIGHT - SPEED_DIGIT_W + SPEED_DIGIT_BEARING;
/// The unit right-aligns on the tenth's ink above it rather than on the column, so
/// the two read as one column of content. Plex's digits are symmetric, so the same
/// bearing trims both sides of the cell.
const SPEED_UNIT_RIGHT: i32 = SPEED_DEC_X + BIG_DIGIT_W - BIG_DIGIT_BEARING;

// The stack is pure const arithmetic, so it can be checked at compile time: the
// speed must not overlap the line beneath it, and that line must stay inside the
// band.
const _: () = assert!(SPEED_CENTER_Y + SPEED_DIGIT_H / 2 < SPEED_INFO_Y - SMALL_CAP_H / 2);
const _: () = assert!(SPEED_INFO_Y + SMALL_CAP_H / 2 <= BAND_TOP.bottom());

/// Two-line label for each time, and the widest line of each -- pinned by
/// `font_metrics_match_the_layout`. The clock is labelled too, for balance with the
/// two beside it.
const CURRENT_TIME_LABEL: [&str; 2] = ["Current", "Time"];
const CURRENT_TIME_LABEL_W: i32 = 66;
/// The heading's label. Only its first line is fixed -- the second is the compass
/// point, which changes -- so the block is sized for the widest of either.
const HEADING_LABEL: &str = "Heading";
const HEADING_LABEL_W: i32 = 72;
const TIME_TO_EMPTY_LABEL: [&str; 2] = ["Time to", "Empty"];
const TIME_TO_EMPTY_LABEL_W: i32 = 66;

/// Full width each time needs: the digits, plus its label.
const TIME_BLOCK_W: i32 = TIME_METRICS.total_w();
const CURRENT_TIME_BLOCK_W: i32 = TIME_BLOCK_W + TIME_LABEL_GAP + CURRENT_TIME_LABEL_W;
/// Three digits, zero-padded like a bearing is written, so the field never changes
/// width -- and the degree sign after them, as the temperatures do it.
const HEADING_DIGITS_W: i32 = 3 * MID_DIGIT_W;
const HEADING_VALUE_W: i32 = HEADING_DIGITS_W + VALUE_UNIT_GAP + SMALL_DEG_W;
const HEADING_BLOCK_W: i32 = HEADING_VALUE_W + TIME_LABEL_GAP + HEADING_LABEL_W;
const TIME_TO_EMPTY_BLOCK_W: i32 = TIME_BLOCK_W + TIME_LABEL_GAP + TIME_TO_EMPTY_LABEL_W;

/// Margin from the screen edges for the outer two times.
const TIME_EDGE_MARGIN: i32 = 12;
/// Left edge of each time block. Placed by anchor rather than by dividing the row
/// into cells: the clock sits at the left edge, the heading is centred on the
/// screen, and time to empty is pushed out to the right edge. Even cells left the
/// outer two looking inset from the edges they should hang off.
const CLOCK_LEFT: i32 = TIME_EDGE_MARGIN;
const HEADING_LEFT: i32 = SCREEN.center_x() - HEADING_BLOCK_W / 2;
const TIME_TO_EMPTY_LEFT: i32 = SCREEN.right() - TIME_EDGE_MARGIN - TIME_TO_EMPTY_BLOCK_W;

// The three blocks must not run into each other.
const _: () = assert!(CLOCK_LEFT + CURRENT_TIME_BLOCK_W < HEADING_LEFT);
const _: () = assert!(HEADING_LEFT + HEADING_BLOCK_W < TIME_TO_EMPTY_LEFT);

/// Split a speed into its whole-number part and its tenth, for drawing either
/// side of the fixed dot.
///
/// Rounds to tenths once so the two halves agree: 21.98 must read `22` and `0`,
/// never `21` and `0`. Absent values give the dashes the old `"--.-"` showed.
/// The 16-point compass name for a bearing in degrees, which must already be
/// normalised to 0..360.
///
/// Sixteen points rather than eight: the extra names cost nothing to read and
/// eight would call everything from 023 to 067 "NE", which is most of a beat.
fn compass_point(degrees: i32) -> &'static str {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    // Each point owns 22.5 deg centred on its bearing, so its boundary sits 11.25
    // before it. Scaled by four to stay in integers: (deg + 11.25) / 22.5.
    POINTS[(((4 * degrees + 45) / 90) % 16) as usize]
}

/// Split a heading into the digits to draw and the compass point to name them.
///
/// Zero-padded to three, the way a bearing is written and spoken, so the field is
/// the same width at 007 as at 127.
fn split_heading(buf: &mut String<16>, value: Option<f32>) -> (&str, &'static str) {
    match value {
        Some(v) => {
            // Half-and-truncate as `split_speed` does it -- `f32::round` is
            // std-only here. `as` saturates, so a NaN lands on 0 rather than
            // wrapping to some plausible-looking bearing, and `rem_euclid` folds
            // 360 back to 0 and tolerates a receiver that reports negatives.
            let degrees = ((v + 0.5) as i32).rem_euclid(360);
            buf.clear();
            write!(buf, "{degrees:03}").ok();
            (buf.as_str(), compass_point(degrees))
        }
        // No fix, or a receiver too slow to have a course yet: the point would be
        // a guess, so it says nothing rather than pointing north.
        None => ("---", ""),
    }
}

fn split_speed(buf: &mut String<8>, value: Option<f32>) -> (&str, &'static str) {
    const TENTHS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

    match value {
        Some(v) => {
            let negative = v.is_sign_negative();
            // `f32::abs` and `f32::round` are std-only and this crate is no_std on
            // the firmware, so do both by hand: add a half and truncate. `as`
            // saturates rather than wrapping, so a NaN or absurd value clamps.
            let magnitude = if negative { -v } else { v };
            let tenths = (magnitude * 10.0 + 0.5) as u32;

            buf.clear();
            if negative {
                // GNSS speed is a magnitude, so this only shows up on a glitch.
                buf.push('-').ok();
            }
            write!(buf, "{}", tenths / 10).ok();
            (buf.as_str(), TENTHS[(tenths % 10) as usize])
        }
        None => ("--", "-"),
    }
}

/// Name the MPPT whose temperature is shown: `MPPT F3` forward, `MPPT R0` aft.
fn fmt_mppt_label(buf: &mut String<16>, id: Option<MpptId>) {
    buf.clear();
    match id {
        Some(MpptId { side, position }) => {
            let side = match side {
                Side::Front => 'F',
                Side::Rear => 'R',
            };
            write!(buf, "MPPT {side}{position}").unwrap();
        }
        // Nothing has reported yet, so there is no unit to name.
        None => buf.push_str("MPPT").unwrap(),
    }
}

/// Describe the GNSS fix state. `None` is nothing received rather than no fix --
/// the receiver being silent is a different thing from it reporting no lock.
fn fmt_gnss_fix(fix: Option<GnssFix>) -> &'static str {
    match fix {
        Some(GnssFix::Fix3D) => "3D fix",
        Some(GnssFix::Fix2D) => "2D fix",
        Some(GnssFix::None) => "No fix",
        None => "---",
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
    // Everything leaving the battery: motor plus peripherals. Negative on the
    // bus, so it renders with a minus sign like the net figure does.
    let power_out = (|| Some(voltage? * (current_motor? + current_peripherals?)))();
    let power_net = (|| Some(voltage? * (current_in? + current_motor? + current_peripherals?)))();

    // Net power is the left column's headline, with its label line beneath in the
    // same shape as the speed. Power in and out follow, smaller, each with a
    // stacked unit/label block.
    draw_text(
        display,
        &FONT_NET,
        HorizontalAlignment::Right,
        NET_VALUE_RIGHT,
        HEADLINE_VALUE_Y,
        fmt_f32(&mut buf, power_net, 0, "----"),
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Right,
        NET_LABEL_RIGHT,
        HEADLINE_LABEL_Y,
        "Net Power",
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Right,
        NET_UNIT_RIGHT,
        HEADLINE_LABEL_Y,
        "W",
    )?;

    for (row, power, label) in [
        (0, power_in, POWER_IN_LABEL),
        (1, power_out, POWER_OUT_LABEL),
    ] {
        draw_reading(
            display,
            &mut buf,
            IN_OUT_RIGHT,
            power,
            "W",
            SMALL_W_W,
            label,
            IN_OUT_CENTERS[row],
            IN_OUT_CENTERS[row] + TEMP_LABEL_OFFSET,
        )?;
    }

    // Centre column: the headline speed, with the GNSS fix state and the unit on
    // a line beneath it. Drawn as three pieces around a static dot so the decimal
    // point holds still as the value crosses 10 km/h.
    // The tenth is drawn at the right column's size, on the same bottom edge as
    // the whole numbers.
    let mut speed_buf: String<8> = String::new();
    let (whole, tenth) = split_speed(&mut speed_buf, data.speed_kmh.get().copied());
    draw_speed_whole(display, whole)?;
    for (x, text) in [(SPEED_DOT_X, "."), (SPEED_DEC_X, tenth)] {
        draw_text(
            display,
            &FONT_BIG,
            HorizontalAlignment::Left,
            x,
            SPEED_DEC_CENTER_Y,
            text,
        )?;
    }
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Left,
        SPEED_FIX_X,
        SPEED_INFO_Y,
        fmt_gnss_fix(data.gnss_fix.get().copied()),
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Right,
        SPEED_UNIT_RIGHT,
        SPEED_INFO_Y,
        "km/h",
    )?;

    // Right column: state of charge at the net power size with a percent sign after
    // it, sitting on the same line as the net power opposite.
    draw_text(
        display,
        &FONT_NET,
        HorizontalAlignment::Right,
        SOC_VALUE_RIGHT,
        HEADLINE_VALUE_Y,
        fmt_f32(
            &mut buf,
            data.battery_state_of_charge.get().copied(),
            0,
            "--",
        ),
    )?;
    draw_text(
        display,
        &FONT_BIG,
        HorizontalAlignment::Left,
        SOC_PCT_X,
        SOC_PCT_CENTER_Y,
        "%",
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Right,
        RIGHT_INNER.right(),
        HEADLINE_LABEL_Y,
        "State of Charge",
    )?;

    // Four temperatures in a 2x2 grid under it. The MPPT label carries the boat
    // position of whichever unit is currently hottest.
    let mut mppt_label: String<16> = String::new();
    let hottest_mppt = data.hottest_mppt();
    fmt_mppt_label(&mut mppt_label, hottest_mppt.map(|(id, _)| id));

    let temps: [(Option<f32>, &str); 4] = [
        (data.motor_temperature(), "Motor"),
        (data.motor_fet_temperature.get().copied(), "Driver"),
        (
            hottest_mppt.map(|(_, celsius)| celsius as f32),
            mppt_label.as_str(),
        ),
        (
            data.hottest_battery_temperature().map(|c| c as f32),
            "Battery",
        ),
    ];

    for (index, (value, label)) in temps.into_iter().enumerate() {
        let cell = TEMP_ROWS[index / 2].cols([1, 1])[index % 2];
        draw_temperature(display, cell, &mut buf, value, label)?;
    }

    draw_icons(display, data)?;
    draw_times(display, data, &mut buf)?;
    draw_version(display)?;
    draw_ip(display, data)?;

    Ok(())
}

/// State of charge below this, in percent, raises the low-charge icon.
const LOW_SOC_PERCENT: f32 = 15.0;
/// Over-temperature limits in °C, one per reading the right column shows. Any one
/// of them raises the single temperature icon -- which reading tripped is left to
/// the numbers themselves.
const MOTOR_TEMP_LIMIT: f32 = 50.0;
const DRIVER_TEMP_LIMIT: f32 = 70.0;
const MPPT_TEMP_LIMIT: i8 = 80;
const BATTERY_TEMP_LIMIT: i8 = 45;

/// The warning icons, in strip order, as raw 1-bit bitmaps generated by
/// `support/png-to-raw.py`: battery, low charge, over-temperature, throttle.
/// A set bit is `BinaryColor::On`, which is this display's background, so the
/// converter clears the bits where the icon has ink.
const ICONS: [&[u8]; ICON_COUNT as usize] = [
    include_bytes!("../../assets/batt48.raw"),
    include_bytes!("../../assets/low48.raw"),
    include_bytes!("../../assets/temp48.raw"),
    include_bytes!("../../assets/throttle48.raw"),
];

/// Draw the icon strip, each icon only while its condition holds.
///
/// The battery and throttle icons carry what the inverted `BAT!`/`THR!` badge used
/// to say, on the same conditions: a stale or missing value does not raise them,
/// since the dashes already show data loss.
///
/// Positions are fixed, so an inactive icon leaves its slot empty rather than the
/// others sliding along -- a warning should always appear in the same place.
/// Left edge of the whole-number cells for a value of this many characters. They
/// are right-aligned, so a shorter value starts further right.
fn speed_whole_left(whole: &str) -> i32 {
    SPEED_INT_RIGHT - whole.chars().count() as i32 * SPEED_DIGIT_W
}

/// The speed's whole numbers, blitted from `SPEED_GLYPHS` and right-aligned on
/// `SPEED_INT_RIGHT`. Every cell is the same width, so the digits line up and the
/// dashes stand in without changing the block's size.
fn draw_speed_whole<D, C>(display: &mut D, whole: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let top = SPEED_CENTER_Y - SPEED_DIGIT_H / 2;
    let mut x = speed_whole_left(whole);

    let mut target = display.color_converted::<BinaryColor>();
    for ch in whole.chars() {
        // Every character `split_speed` can emit is in the blob; anything else
        // would be a bug here rather than bad data, so skipping is enough.
        if let Some(index) = SPEED_GLYPH_ORDER.find(ch) {
            let start = index * SPEED_GLYPH_BYTES;
            let raw = ImageRaw::<BinaryColor>::new(
                &SPEED_GLYPHS[start..start + SPEED_GLYPH_BYTES],
                SPEED_DIGIT_W as u32,
            );
            Image::new(&raw, Point::new(x, top)).draw(&mut target)?;
        }
        x += SPEED_DIGIT_W;
    }
    Ok(())
}

/// Which icons the current data raises, in strip order.
///
/// The battery and throttle conditions are what the inverted `BAT!`/`THR!` badge
/// used to report. A stale or missing value raises nothing: `get` returns `None`
/// once a value times out, so nothing latches the last warning, and the dashes
/// already show data loss.
fn icon_conditions(data: &DisplayData) -> [bool; ICON_COUNT as usize] {
    let battery_abnormal = !matches!(data.battery_state.get(), None | Some(BatteryState::On))
        || !matches!(
            data.battery_charge_state.get(),
            None | Some(ChargeState::FetOn)
        )
        || !matches!(
            data.battery_discharge_state.get(),
            None | Some(DischargeState::On)
        );

    let low_charge = data
        .battery_state_of_charge
        .get()
        .is_some_and(|soc| *soc < LOW_SOC_PERCENT);

    let over_temperature = data
        .motor_temperature()
        .is_some_and(|t| t > MOTOR_TEMP_LIMIT)
        || data
            .motor_fet_temperature
            .get()
            .is_some_and(|t| *t > DRIVER_TEMP_LIMIT)
        || data
            .hottest_mppt()
            .is_some_and(|(_, celsius)| celsius > MPPT_TEMP_LIMIT)
        || data
            .hottest_battery_temperature()
            .is_some_and(|celsius| celsius > BATTERY_TEMP_LIMIT);

    let throttle_error = data.throttle_errors.get().is_some_and(|e| e.has_error());

    [
        battery_abnormal,
        low_charge,
        over_temperature,
        throttle_error,
    ]
}

/// Draw the icon strip, each icon only while its condition holds.
///
/// Positions are fixed, so an inactive icon leaves its slot empty rather than the
/// others sliding along -- a warning should always appear in the same place.
fn draw_icons<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    // ImageRaw yields BinaryColor, and the display's colour is whatever the caller
    // uses, so convert on the way through.
    let mut target = display.color_converted::<BinaryColor>();
    for (index, (bytes, active)) in ICONS.iter().zip(icon_conditions(data)).enumerate() {
        if !active {
            continue;
        }
        let raw = ImageRaw::<BinaryColor>::new(bytes, ICON_SIZE as u32);
        let x = ICON_STRIP_X + index as i32 * (ICON_SIZE + ICON_GAP);
        Image::new(&raw, Point::new(x, ICON_STRIP_Y)).draw(&mut target)?;
    }
    Ok(())
}

/// Bottom row: time of day, heading, time to empty.
fn draw_times<D, C>(
    display: &mut D,
    data: &DisplayData,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let time = data
        .time
        .get()
        .map(|t| ((t.hours + cet_offset_hours(t)) % 24, t.minutes, t.seconds));

    draw_time(
        display,
        &FONT_MID,
        TIME_METRICS,
        CLOCK_LEFT,
        TIME_CENTER_Y,
        fmt_hms_groups(buf, time),
        Some((CURRENT_TIME_LABEL, CURRENT_TIME_LABEL_W)),
    )?;
    // Heading takes the centre slot, where the race time used to draw dashes it
    // had no source for. Drawn as digits at the times' own size so the row still
    // reads as one line, with the compass point under the label: the number is
    // what you steer to, the point is what tells you at a glance which way that is.
    let (degrees, point) = split_heading(buf, data.heading_deg.get().copied());
    draw_text(
        display,
        &FONT_MID,
        HorizontalAlignment::Right,
        HEADING_LEFT + HEADING_DIGITS_W,
        TIME_CENTER_Y,
        degrees,
    )?;
    // The unit sits on the digits' baseline, as the temperatures' "°C" does.
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Left,
        HEADING_LEFT + HEADING_DIGITS_W + VALUE_UNIT_GAP,
        TIME_CENTER_Y + MID_DIGIT_H / 2 - SMALL_CAP_H / 2,
        "°",
    )?;
    draw_stacked_label(
        display,
        HEADING_LEFT + HEADING_VALUE_W + TIME_LABEL_GAP,
        TIME_CENTER_Y,
        HEADING_LABEL,
        point,
    )?;
    // Derived on ingest from the smoothed draw and the state of charge, and
    // dashes whenever that could not be worked out -- see `update_endurance`.
    let time_to_empty = data
        .battery_time_to_empty
        .get()
        .map(|&seconds| hms_from_seconds(seconds));
    draw_time(
        display,
        &FONT_MID,
        TIME_METRICS,
        TIME_TO_EMPTY_LEFT,
        TIME_CENTER_Y,
        fmt_hms_groups(buf, time_to_empty),
        Some((TIME_TO_EMPTY_LABEL, TIME_TO_EMPTY_LABEL_W)),
    )
}

fn draw_version<D, C>(display: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
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

    draw_stamp(
        display,
        COL_MID.center_x(),
        Alignment::Center,
        version.as_str(),
    )
}

/// The data logger's WiFi IP, on the build stamp's line but in the left corner,
/// flush with the screen edge like the net power under it. Drawn only while the
/// value is fresh: the Pi broadcasts it every second (and the std front-ends also
/// query it locally), so no WiFi or no frames makes it disappear rather than
/// leave a stale address up.
fn draw_ip<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let Some(address) = data.ip_address.get() else {
        return Ok(());
    };

    let mut ip: String<24> = String::new();
    write!(&mut ip, "IP: {address}").unwrap();

    draw_stamp(display, COL_LEFT.x, Alignment::Left, ip.as_str())
}
#[cfg(test)]
mod tests {
    use super::*;
    use eoi_can_decoder::ThrottleErrors;

    /// The strap-to-label mapping, checked against the documented CAN IDs.
    /// `node = 64 + strap`, `ID = (node << 4) | packet`, bit 3 of the strap selects
    /// the side. The decoder reports the strap, which is what reaches us here.
    #[test]
    fn fmt_mppt_label_names_both_sides() {
        let mut buf: String<16> = String::new();

        // Every strap, against the documented table.
        for (strap, expected, id_base) in [
            (0u8, "MPPT R0", 0x400u16),
            (1, "MPPT R1", 0x410),
            (2, "MPPT R2", 0x420),
            (3, "MPPT R3", 0x430),
            (4, "MPPT R4", 0x440),
            (5, "MPPT R5", 0x450),
            (6, "MPPT R6", 0x460),
            (7, "MPPT R7", 0x470),
            (8, "MPPT F0", 0x480),
            (9, "MPPT F1", 0x490),
            (10, "MPPT F2", 0x4A0),
            (11, "MPPT F3", 0x4B0),
            (12, "MPPT F4", 0x4C0),
            (13, "MPPT F5", 0x4D0),
            (14, "MPPT F6", 0x4E0),
            (15, "MPPT F7", 0x4F0),
        ] {
            let (side, position) = mppt_layout::gan_side_and_position(strap);
            fmt_mppt_label(&mut buf, Some(MpptId { side, position }));
            assert_eq!(buf.as_str(), expected, "strap {strap}");

            // Cross-check the strap against the documented base CAN ID.
            assert_eq!(
                ((64 + strap as u16) << 4),
                id_base,
                "strap {strap} does not map to {id_base:#05X}"
            );
        }

        fmt_mppt_label(&mut buf, None);
        assert_eq!(buf.as_str(), "MPPT");
    }

    #[test]
    fn fmt_gnss_fix_covers_every_state() {
        assert_eq!(fmt_gnss_fix(Some(GnssFix::Fix3D)), "3D fix");
        assert_eq!(fmt_gnss_fix(Some(GnssFix::Fix2D)), "2D fix");
        assert_eq!(fmt_gnss_fix(Some(GnssFix::None)), "No fix");
        // Nothing received at all, which is not the same as a reported no-fix.
        assert_eq!(fmt_gnss_fix(None), "---");

        // The label is anchored to the last whole digit, so it does not move with the
        // value. Every label it can show has to clear the unit beside it.
        for label in ["3D fix", "2D fix", "No fix", "---"] {
            let right = SPEED_FIX_X + width(&FONT_SMALL, label);
            assert!(
                right < SPEED_UNIT_RIGHT - width(&FONT_SMALL, "km/h"),
                "{label:?} ends at {right}, into the unit"
            );
        }
        // It sits under the digits, so it starts after the widest value's left edge.
        assert!(
            SPEED_FIX_X > speed_whole_left("-0"),
            "the fix label starts left of the widest speed"
        );
    }

    /// The `Cell` splits must tile their parent exactly -- no gap, no overlap, no
    /// rounding loss -- or cells drift out of step with the content placed in them.
    #[test]
    fn grid_splits_tile_their_parent() {
        assert_eq!(BAND_TOP.y, SCREEN.y);
        assert_eq!(BAND_TOP.bottom() + ROW_BOTTOM_H, SCREEN.bottom());

        for (name, cells) in [("top columns", TOP_COLS.as_slice())] {
            assert_eq!(cells[0].x, 0, "{name} start at the left edge");
            assert_eq!(
                cells[cells.len() - 1].right(),
                SCREEN.right(),
                "{name} end at the right edge"
            );
            for pair in cells.windows(2) {
                assert_eq!(pair[0].right(), pair[1].x, "{name} leave no seam");
            }
        }

        let rows = COL_LEFT.rows::<3>();
        assert_eq!(rows[0].y, COL_LEFT.y);
        assert_eq!(rows[2].bottom(), COL_LEFT.bottom());

        // The 35/30/35 split: outer columns wider than the centre.
        const { assert!(COL_LEFT.w > COL_MID.w && COL_RIGHT.w > COL_MID.w) };
    }

    /// The strings this layout reserves width for, checked against the fonts as
    /// built. The shared `font_metrics_match_their_consts` pins the glyph
    /// advances; these are widths of text only this screen shows.
    #[test]
    fn label_widths_match_their_consts() {
        assert_eq!(
            width(&FONT_SMALL, "Net Power"),
            NET_LABEL_W,
            "FONT_SMALL \"Net Power\" advance changed; update NET_LABEL_W"
        );
        // The heading's label block holds two lines that are not both fixed: the
        // word, and whichever compass point is longest. Either one overflowing
        // would run the label into the block beside it.
        for line in [
            "N",
            "NNE",
            "NE",
            "ENE",
            "E",
            "ESE",
            "SE",
            "SSE",
            "S",
            "SSW",
            "SW",
            "WSW",
            "W",
            "WNW",
            "NW",
            "NNW",
            HEADING_LABEL,
        ] {
            let w = width(&FONT_SMALL, line);
            assert!(
                w <= HEADING_LABEL_W,
                "\"{line}\" is {w}px, past the {HEADING_LABEL_W}px the heading label reserves"
            );
        }
        assert_eq!(
            width(&FONT_SMALL, HEADING_LABEL),
            HEADING_LABEL_W,
            "\"{HEADING_LABEL}\" no longer sets the label width; update HEADING_LABEL_W"
        );

        // The two-line time labels size their cells, so their widest line is layout.
        for (lines, w) in [
            (CURRENT_TIME_LABEL, CURRENT_TIME_LABEL_W),
            (TIME_TO_EMPTY_LABEL, TIME_TO_EMPTY_LABEL_W),
        ] {
            let widest = lines.iter().map(|l| width(&FONT_SMALL, l)).max().unwrap();
            assert_eq!(
                widest, w,
                "the widest line of {lines:?} is {widest}px, not the {w}px recorded"
            );
        }
    }

    #[test]
    fn split_heading_names_the_point_it_is_in() {
        let mut buf: String<16> = String::new();
        assert_eq!(split_heading(&mut buf, Some(0.0)), ("000", "N"));
        assert_eq!(split_heading(&mut buf, Some(127.0)), ("127", "SE"));
        // A point owns 11.25 deg either side of its bearing, so the name changes
        // between 11 and 12, not at 22.
        assert_eq!(split_heading(&mut buf, Some(11.0)), ("011", "N"));
        assert_eq!(split_heading(&mut buf, Some(12.0)), ("012", "NNE"));
        // And back onto north the long way round, which is the wrap the modulo
        // exists for.
        assert_eq!(split_heading(&mut buf, Some(348.0)), ("348", "NNW"));
        assert_eq!(split_heading(&mut buf, Some(349.0)), ("349", "N"));
        // Rounds to 360, which is 000 -- never a fourth digit.
        assert_eq!(split_heading(&mut buf, Some(359.7)), ("000", "N"));
        // No fix: dashes, and no point rather than a guessed one.
        assert_eq!(split_heading(&mut buf, None), ("---", ""));
    }

    /// Values no receiver should send, which must still land on the dial rather
    /// than off it -- or panic the render by indexing past the sixteen points.
    #[test]
    fn split_heading_survives_nonsense() {
        let mut buf: String<16> = String::new();
        // NaN casts to 0, which is a real bearing and the only sane one to pick.
        assert_eq!(split_heading(&mut buf, Some(f32::NAN)), ("000", "N"));
        // Truncation is towards zero, so a negative lands a degree off -- it is a
        // glitch path, and being on the right point is what matters.
        assert_eq!(split_heading(&mut buf, Some(-90.0)), ("271", "W"));
        // The saturating casts put absurd values somewhere arbitrary on the dial.
        // Which point is not worth pinning; staying on the dial is.
        for absurd in [f32::INFINITY, f32::NEG_INFINITY, 1.0e9, -1.0e9] {
            let (degrees, point) = split_heading(&mut buf, Some(absurd));
            let value: i32 = degrees.parse().expect("three digits");
            assert!(
                (0..360).contains(&value),
                "{absurd} drew {degrees}, which is off the dial"
            );
            assert_eq!(point, compass_point(value));
        }
    }

    /// Every bearing has a name, and the index that finds it never leaves the
    /// table -- this runs on a panel that must not die on a stray frame.
    #[test]
    fn every_degree_lands_on_a_point() {
        let mut seen = heapless::Vec::<&str, 16>::new();
        for degrees in 0..360 {
            let point = compass_point(degrees);
            if !seen.contains(&point) {
                seen.push(point)
                    .expect("no more than sixteen distinct points");
            }
        }
        assert_eq!(seen.len(), 16, "not every compass point is reachable");
    }

    #[test]
    fn split_speed_rounds_both_halves_together() {
        let mut buf: String<8> = String::new();
        assert_eq!(split_speed(&mut buf, Some(0.0)), ("0", "0"));
        assert_eq!(split_speed(&mut buf, Some(2.54)), ("2", "5"));
        assert_eq!(split_speed(&mut buf, Some(21.65)), ("21", "7"));
        // The whole part has to follow the rounding of the tenth.
        assert_eq!(split_speed(&mut buf, Some(21.98)), ("22", "0"));
        assert_eq!(split_speed(&mut buf, Some(99.94)), ("99", "9"));
        // Only reachable via a bad frame, but it must not render as "0.2".
        assert_eq!(split_speed(&mut buf, Some(-0.2)), ("-0", "2"));
        assert_eq!(split_speed(&mut buf, None), ("--", "-"));
    }

    /// `render_aligned` clips rather than erroring, so a value that outgrows its
    /// space produces no warning at all. The strings are the widest each row
    /// reaches in service, not the widest `fmt_f32` could emit.
    #[test]
    fn widest_values_fit_their_cells() {
        // The net power figure is right-justified so the widest value -- a minus and
        // four digits -- lands exactly on the screen's left edge.
        assert_eq!(
            NET_FIELD_W,
            width(&FONT_NET, "-2000"),
            "the reserved net power field no longer matches a minus and four digits"
        );
        assert_eq!(
            NET_VALUE_RIGHT - width(&FONT_NET, "-2000"),
            COL_LEFT.x,
            "the widest net power should sit flush against the screen edge"
        );
        // Nothing wider than that ever renders, but check the anchor stays inside
        // the column so the figure cannot reach the centre column's content.
        assert!(
            NET_VALUE_RIGHT <= COL_LEFT.right(),
            "the net power anchor is past its column"
        );

        // The reserved in/out field must hold a minus and four digits, or a long
        // value would run into the unit beside it.
        let widest_in_out = width(&FONT_MID, "-2000");
        assert!(
            widest_in_out <= POWER_FIELD_W,
            "\"-2000\" is {widest_in_out}px, wider than the {POWER_FIELD_W}px reserved"
        );
        // Value, unit and label all end on IN_OUT_RIGHT and grow leftwards from it.
        assert!(
            IN_OUT_RIGHT <= LEFT_INNER.right(),
            "in/out readings end at {IN_OUT_RIGHT}, past the column"
        );
        for label in [POWER_IN_LABEL, POWER_OUT_LABEL] {
            let left = IN_OUT_RIGHT - width(&FONT_SMALL, label);
            assert!(
                left >= COL_LEFT.x,
                "{label:?} starts at {left}, off the left of the screen"
            );
        }

        // Right column: the state of charge and its percent sign are right-aligned
        // as a pair. "State of Charge" shares the label line with nothing else, so
        // it only has to fit the column.
        let soc_left = SOC_VALUE_RIGHT - width(&FONT_NET, "100");
        assert!(
            soc_left >= RIGHT_INNER.x,
            "state of charge starts at {soc_left}, left of its column at {}",
            RIGHT_INNER.x
        );
        assert!(
            SOC_PCT_X + BIG_PCT_W <= RIGHT_INNER.right(),
            "the percent sign runs past the column edge"
        );
        const { assert!(SOC_VALUE_RIGHT < SOC_PCT_X) };
        // The right column's labels are right-aligned on the data above them, so
        // they grow leftwards and must not escape the column.
        let soc_label_left = RIGHT_INNER.right() - width(&FONT_SMALL, "State of Charge");
        assert!(
            soc_label_left >= RIGHT_INNER.x,
            "\"State of Charge\" starts at {soc_label_left}, left of the column at {}",
            RIGHT_INNER.x
        );

        // Temperature grid: the digits, the degree sign and the widest label all
        // grow leftwards from each cell's right edge.
        let widest_temp = width(&FONT_MID, "108");
        assert!(
            widest_temp <= TEMP_FIELD_W,
            "\"108\" is {widest_temp}px, wider than the {TEMP_FIELD_W}px reserved"
        );
        for (cell_name, cell) in [
            ("left", TEMP_ROWS[0].cols([1, 1])[0]),
            ("right", TEMP_ROWS[0].cols([1, 1])[1]),
        ] {
            let digits_left = cell.right() - SMALL_DEG_C_W - VALUE_UNIT_GAP - TEMP_FIELD_W;
            assert!(
                digits_left >= cell.x,
                "{cell_name} temperature digits start at {digits_left}, left of the \
                 cell at {}",
                cell.x
            );
            // The MPPT label carries a boat position, so it is the longest.
            for label in ["Motor", "Driver", "MPPT 11", "Battery"] {
                let label_left = cell.right() - width(&FONT_SMALL, label);
                assert!(
                    label_left >= cell.x,
                    "{cell_name} label {label:?} starts at {label_left}, left of the cell"
                );
            }
        }

        // The right-hand column of the grid ends on the column edge, which is where
        // the whole block is meant to hang from.
        assert_eq!(
            TEMP_ROWS[0].cols([1, 1])[1].right(),
            RIGHT_INNER.right(),
            "the temperature grid does not reach the column's right edge"
        );
    }

    /// Icon order is battery, low charge, over-temperature, throttle. Each threshold
    /// is checked on the wrong side too -- a flipped comparison is exactly the kind
    /// of mistake that would leave a warning permanently on or permanently off.
    #[test]
    fn icon_conditions_follow_their_thresholds() {
        const NONE: [bool; 4] = [false, false, false, false];
        const BATTERY: usize = 0;
        const LOW: usize = 1;
        const TEMP: usize = 2;
        const THROTTLE: usize = 3;

        // Nothing received yet: no icon should be raised.
        let data = DisplayData::default();
        assert_eq!(
            icon_conditions(&data),
            NONE,
            "an empty display raises icons"
        );

        // A healthy boat: every value present, all inside limits.
        let healthy = || {
            let mut d = DisplayData::default();
            d.battery_state.update(BatteryState::On);
            d.battery_charge_state.update(ChargeState::FetOn);
            d.battery_discharge_state.update(DischargeState::On);
            d.battery_state_of_charge.update(LOW_SOC_PERCENT + 1.0);
            d.motor_ntc_temperature.update(Some(MOTOR_TEMP_LIMIT - 1.0));
            d.motor_fet_temperature.update(DRIVER_TEMP_LIMIT - 1.0);
            d.mppt_temperatures[0].update(MPPT_TEMP_LIMIT - 1);
            d.battery_temperatures[0].update(BATTERY_TEMP_LIMIT - 1);
            d
        };
        assert_eq!(
            icon_conditions(&healthy()),
            NONE,
            "a healthy boat raises icons"
        );

        // Each limit, one at a time.
        let mut d = healthy();
        d.battery_state_of_charge.update(LOW_SOC_PERCENT - 0.1);
        assert!(icon_conditions(&d)[LOW], "low charge missed");

        for (name, slot) in [
            ("motor", TEMP),
            ("driver", TEMP),
            ("mppt", TEMP),
            ("battery thermistor", TEMP),
        ] {
            let mut d = healthy();
            match name {
                "motor" => d.motor_ntc_temperature.update(Some(MOTOR_TEMP_LIMIT + 0.1)),
                "driver" => d.motor_fet_temperature.update(DRIVER_TEMP_LIMIT + 0.1),
                "mppt" => d.mppt_temperatures[0].update(MPPT_TEMP_LIMIT + 1),
                _ => d.battery_temperatures[0].update(BATTERY_TEMP_LIMIT + 1),
            }
            assert!(icon_conditions(&d)[slot], "{name} over-temperature missed");
        }

        // The battery and throttle conditions inherited from the old badge.
        let mut d = healthy();
        d.battery_charge_state.update(ChargeState::Error);
        assert!(icon_conditions(&d)[BATTERY], "battery error missed");

        let mut d = healthy();
        d.throttle_errors.update(ThrottleErrors {
            deadman_missing: true,
            ..Default::default()
        });
        assert!(icon_conditions(&d)[THROTTLE], "throttle error missed");
    }

    /// The VESC also reports a motor temperature and it is broken, so the Motor cell
    /// must never pick it up -- not as a fallback and not while the node is quiet.
    #[test]
    fn motor_temperature_comes_only_from_the_standalone_node() {
        let mut d = DisplayData::default();
        assert_eq!(d.motor_temperature(), None, "nothing reporting");

        d.vesc_motor_temperature.update(58.7);
        assert_eq!(
            d.motor_temperature(),
            None,
            "the VESC's broken reading leaked into the Motor cell"
        );

        d.motor_ntc_temperature.update(Some(61.4));
        assert_eq!(d.motor_temperature(), Some(61.4), "the node should show");

        d.motor_ntc_temperature.update(None);
        assert_eq!(
            d.motor_temperature(),
            None,
            "a node fault should show dashes"
        );
    }

    /// The frames themselves, so the ID and scaling are covered end to end and not
    /// only through a hand-set field.
    #[test]
    fn motor_ntc_frames_reach_the_motor_cell() {
        use embedded_can::{Id, StandardId};
        use eoi_can_decoder::can_frame::CanFrame;
        use eoi_can_decoder::parse_eoi_can_data;

        let ingest = |d: &mut DisplayData, bytes: &[u8]| {
            let frame =
                CanFrame::from_encoded(Id::Standard(StandardId::new(0x219).unwrap()), bytes);
            d.ingest_eoi_can_data(parse_eoi_can_data(&frame).expect("0x219 should decode"));
        };

        let mut d = DisplayData::default();
        ingest(&mut d, &[0xEB, 0x00, 0x00, 0x01]); // 235 dd
        assert_eq!(d.motor_temperature(), Some(23.5));

        ingest(&mut d, &[0x00, 0x80, 0x01, 0x02]); // sentinel, sensor open
        assert_eq!(d.motor_temperature(), None);

        ingest(&mut d, &[0xEB, 0x00, 0x08, 0x03]); // settling, but a real reading
        assert_eq!(d.motor_temperature(), Some(23.5));
    }

    /// The icon strip is deliberately wider than the centre column, so what matters
    /// is that it stays on screen, stays centred, and every bitmap is the size the
    /// layout assumes.
    #[test]
    fn icon_strip_is_centred_and_sized() {
        // 48px rows pack into exactly 6 bytes, so a bitmap is 48 * 6 bytes. A
        // wrong-sized blob would silently render as a different height.
        let expected = (ICON_SIZE * ICON_SIZE / 8) as usize;
        for (index, bytes) in ICONS.iter().enumerate() {
            assert_eq!(
                bytes.len(),
                expected,
                "icon {index} is {} bytes, not the {expected} a {ICON_SIZE}px square \
                 needs -- regenerate with support/png-to-raw.py",
                bytes.len()
            );
        }

        // Centred on the column it sits under, plus its deliberate nudge, even
        // though it overhangs the column either side.
        let strip_center = ICON_STRIP_X + ICON_STRIP_W / 2;
        let expected = COL_MID.center_x() + ICON_SHIFT;
        assert!(
            (strip_center - expected).abs() <= 1,
            "the strip centres on {strip_center}, not {expected}"
        );

        // On screen, and clear of the fix/unit line and the badge -- the vertical
        // fit is a compile-time check by `ICON_STRIP_Y`.
        assert!(
            ICON_STRIP_X >= 0 && ICON_STRIP_X + ICON_STRIP_W <= SCREEN.right(),
            "the strip spans {ICON_STRIP_X}..{}, off screen",
            ICON_STRIP_X + ICON_STRIP_W
        );
    }

    /// The centre column stacks the build stamp, the speed, and the icon strip, so
    /// each has to stay out of the next one's way.
    #[test]
    fn centre_column_stack_does_not_overlap() {
        let speed_top = SPEED_CENTER_Y - SPEED_DIGIT_H / 2;
        assert!(
            VERSION_BASELINE_Y < speed_top,
            "the build stamp baseline y={VERSION_BASELINE_Y} runs into the speed at \
             y={speed_top}"
        );

        // The icons sit at the bottom of the band, so the gap above them is what is
        // left of the space `SPEED_LIFT` opened up. Zero means the lift is too small.
        let gap = ICON_STRIP_Y - ICON_BAND_TOP;
        assert!(
            gap >= 2,
            "only {gap}px between the fix/unit line and the icons -- raise SPEED_LIFT"
        );
    }

    /// The IP shares the stamp line with the build stamp, so the widest address
    /// must end before the centred version text can begin.
    #[test]
    fn ip_stamp_clears_the_version_stamp() {
        // `FONT_4X6` advances 4px per character.
        let ip_right = COL_LEFT.x + 4 * "IP: 255.255.255.255".len() as i32;
        // The version is centred on the middle column; even one as wide as the
        // whole column stays right of this.
        let version_left_bound = COL_MID.center_x() - COL_MID.w / 2;
        assert!(
            ip_right < version_left_bound,
            "the IP stamp reaches x={ip_right}, into the version stamp's column at \
             x={version_left_bound}"
        );
    }

    /// The speed's whole numbers come from a blob of fixed-size cells, so the blob
    /// has to agree with the cell geometry the layout computes from. A mismatch
    /// would render the digits at the wrong height or slice them apart mid-glyph.
    #[test]
    fn speed_glyphs_match_their_cells() {
        assert_eq!(
            SPEED_GLYPHS.len(),
            SPEED_GLYPH_ORDER.chars().count() * SPEED_GLYPH_BYTES,
            "the speed blob holds {} bytes, not the {} that {} cells of {}x{} need \
             -- regenerate with support/ttf-digits-to-raw.py",
            SPEED_GLYPHS.len(),
            SPEED_GLYPH_ORDER.chars().count() * SPEED_GLYPH_BYTES,
            SPEED_GLYPH_ORDER.chars().count(),
            SPEED_DIGIT_W,
            SPEED_DIGIT_H
        );

        // Every character `split_speed` can produce must be in the blob, or it would
        // silently vanish from the display.
        for ch in "0123456789-".chars() {
            assert!(
                SPEED_GLYPH_ORDER.contains(ch),
                "{ch:?} can reach the speed but has no glyph"
            );
        }
    }

    /// The speed is wider than the centre column, which is fine with no rules drawn.
    /// What matters is that it clears the content either side of it, and that its
    /// pieces sit in the right order.
    #[test]
    fn speed_pieces_clear_their_neighbours() {
        // Left edge of the widest whole part, right edge of the tenth.
        let block_left = SPEED_INT_RIGHT - 2 * SPEED_DIGIT_W;
        let block_right = SPEED_DEC_X + BIG_DIGIT_W;

        assert!(
            block_left > NET_VALUE_RIGHT,
            "speed starts at {block_left}, into the net power ending at {NET_VALUE_RIGHT}"
        );
        assert!(
            block_right < SOC_VALUE_LEFT,
            "speed ends at {block_right}, into the state of charge starting at \
             {SOC_VALUE_LEFT}"
        );

        // The bearings the dot spacing is computed from must match the artwork, or
        // SPEED_DOT_GAP stops meaning ink-to-ink air.
        let row = SPEED_GLYPH_BYTES / SPEED_DIGIT_H as usize;
        let tightest = (0..10)
            .map(|d| {
                let glyph = &SPEED_GLYPHS[d * SPEED_GLYPH_BYTES..(d + 1) * SPEED_GLYPH_BYTES];
                let right = (0..SPEED_DIGIT_W as usize)
                    .rfind(|x| {
                        (0..SPEED_DIGIT_H as usize)
                            .any(|y| glyph[y * row + x / 8] >> (7 - (x % 8)) & 1 == 0)
                    })
                    .expect("every digit has ink");
                SPEED_DIGIT_W - 1 - right as i32
            })
            .min()
            .unwrap();
        assert_eq!(
            tightest, SPEED_DIGIT_BEARING,
            "the narrowest digit bearing is {tightest}px, not the {SPEED_DIGIT_BEARING}px \
             recorded -- the dot would sit closer to some digits than SPEED_DOT_GAP"
        );
        for (name, glyph, bearing) in [
            ("dot", ".", BIG_DOT_BEARING),
            ("digit", "0", BIG_DIGIT_BEARING),
        ] {
            let x = FONT_BIG
                .get_rendered_dimensions(glyph, Point::zero(), VerticalPosition::Center)
                .unwrap()
                .bounding_box
                .expect("glyph renders")
                .top_left
                .x;
            assert_eq!(x, bearing, "FONT_BIG {name} left bearing changed");
        }

        // The fix label and unit share the line under the speed; the label's
        // clearance is checked where it sits furthest right, in
        // `fmt_gnss_fix_covers_every_state`.
        const { assert!(SPEED_UNIT_RIGHT < SOC_VALUE_LEFT) };
    }

    /// Shrinking the top band or raising a font size must not push ink out of a
    /// cell or across the band boundary into the times.
    #[test]
    fn rows_fit_the_top_band() {
        // Both columns' headline values share one line and must stay on screen.
        let headline_top = HEADLINE_VALUE_Y - NET_DIGIT_H / 2;
        assert!(
            headline_top >= 0,
            "headline values start at y={headline_top}, off the top of the screen"
        );
        // The headline's internal spacing and its clearance from the body below are
        // compile-time checks; see the const asserts by `HEADLINE_STACK`.

        // Temperature grid: value and label must fit each row, and the grid must sit
        // at the bottom of the band without crossing it. Its clearance from the
        // headline above is a compile-time check by `TEMP_GRID`.
        for (i, row) in TEMP_ROWS.iter().enumerate() {
            assert!(
                TEMP_BLOCK_H <= row.h,
                "temperature row {i} is {}px, too short for a {TEMP_BLOCK_H}px block",
                row.h
            );
        }
        assert!(
            TEMP_GRID.bottom() < TIME_TOP_Y,
            "the temperature grid reaches the times at {TIME_TOP_Y}"
        );

        // "Net Power" is right-justified and pulled left of the value's edge; it may
        // sit outside the column padding but not off the screen, and it shares its
        // line with the "W", which is itself pulled a digit in from the edge.
        let net_label_left = NET_LABEL_RIGHT - width(&FONT_SMALL, "Net Power");
        assert!(
            net_label_left >= COL_LEFT.x,
            "\"Net Power\" starts at {net_label_left}, off the left of the screen"
        );
        let unit_left = NET_UNIT_RIGHT - width(&FONT_SMALL, "W");
        assert!(
            NET_LABEL_RIGHT + 8 <= unit_left,
            "\"Net Power\" ends at {NET_LABEL_RIGHT}, too close to the W at {unit_left}"
        );

        // The in/out values and their two-line label blocks must fit their rows.
        let mid_h = digit_height(&FONT_MID);
        let stacked_h = 2 * SMALL_CAP_H + STACK_LABEL_GAP;
        assert!(
            mid_h <= IN_OUT_BLOCK_H && stacked_h <= IN_OUT_BLOCK_H,
            "an in/out block is {IN_OUT_BLOCK_H}px, too short for a {mid_h}px value \
             and a {stacked_h}px label block"
        );
        // The point of pinning them together: read across and both rows line up.
        assert_eq!(
            IN_OUT_CENTERS, TEMP_VALUE_CENTERS,
            "the in/out rows no longer line up with the temperatures"
        );
        assert!(
            LEFT_BODY.bottom() <= BAND_TOP.bottom(),
            "in/out rows run past the band into the times"
        );

        // The centre stack is checked at compile time; see the const asserts by
        // `MID_STACK`.
    }

    /// The bottom row has to be tall enough for the clock and wide enough for each
    /// time plus its label.
    #[test]
    fn times_fit_the_bottom_row() {
        // The times hang off the bottom of the screen, so what bounds them is the
        // screen edge below and the band above -- not a row they sit inside.
        let time_top = TIME_CENTER_Y - TIME_BLOCK_H / 2;
        let time_bottom = TIME_CENTER_Y + TIME_BLOCK_H / 2;
        assert!(
            time_bottom <= SCREEN.bottom(),
            "the times end at {time_bottom}, off the bottom of the screen"
        );
        assert!(
            time_top > BAND_TOP.bottom(),
            "the times start at {time_top}, up into the band ending at {}",
            BAND_TOP.bottom()
        );
        assert!(
            digit_height(&FONT_MID) <= TIME_BLOCK_H,
            "the digits are taller than the block they are centred in"
        );

        // The whole point of the slotted layout: the block is the same width
        // whatever it holds, because the colons are placed rather than typeset. A
        // dash is narrower than a digit, so the plain string is not.
        assert_eq!(
            TIME_METRICS.total_w(),
            width(&FONT_MID, "23:59:59"),
            "the time block arithmetic disagrees with the font"
        );
        assert!(
            width(&FONT_MID, "--:--:--") < width(&FONT_MID, "23:59:59"),
            "dashes are no longer narrower than digits, so the slotted layout is \
             guarding nothing"
        );

        // The blocks are anchored rather than cell-divided, so what matters is that
        // they stay on screen. Whether they collide is a compile-time check by the
        // anchors themselves.
        for (name, left, block_w) in [
            ("clock", CLOCK_LEFT, CURRENT_TIME_BLOCK_W),
            ("heading", HEADING_LEFT, HEADING_BLOCK_W),
            ("time to empty", TIME_TO_EMPTY_LEFT, TIME_TO_EMPTY_BLOCK_W),
        ] {
            assert!(
                left >= 0,
                "{name} starts at {left}, off the left of the screen"
            );
            let right = left + block_w;
            assert!(
                right <= SCREEN.right(),
                "{name} ends at {right}, past the right edge at {}",
                SCREEN.right()
            );
        }

        // The heading should read as centred on the screen.
        let heading_center = HEADING_LEFT + HEADING_BLOCK_W / 2;
        assert!(
            (heading_center - SCREEN.center_x()).abs() <= 1,
            "the heading block centres on {heading_center}, not the screen centre {}",
            SCREEN.center_x()
        );
    }
}
