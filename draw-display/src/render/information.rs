//! The information screen: what the foiling board shows when nobody is tuning.
//!
//! Geometry only -- the `Cell` model, the row grid, the fonts and every widget
//! come from [`super`], exactly as [`super::foiling`] uses them. It shares that
//! screen's row pitch on purpose, so a helm switching between the two is reading
//! the same lines in the same places.
//!
//! Three blocks tile the full width. The MPPT table takes the left half because
//! it is the only part whose length is not known until the bus answers; the two
//! side columns carry the rest of the boat, grouped under the subsystem that
//! reports them.
//!
//! Two ideas do the work here:
//!
//! - **Units live in the heading, not the cell.** A table column is one quantity,
//!   so repeating "V" down eleven rows costs a column's width to say nothing new.
//!   The side columns are the other way round -- each row is a different quantity,
//!   so there the unit sits beside the value.
//! - **The MPPT table is as long as the bus is.** Rows are filled from whichever
//!   ID straps are currently reporting, in strap order, which is CAN address
//!   order: a GaN unit's ID is `(64 + strap) << 4 | packet`. A unit that is
//!   unplugged leaves no gap, and one plugged in without a place in `LAYOUT`
//!   still appears.
//!
//! The display is a pure renderer, as everywhere else: every value here is drawn
//! as received, and an absent one draws as dashes rather than as a zero.

use core::fmt::Write;

use heapless::String;
use mppt_layout::{gan_side_and_position, Side, GAN_STRAP_COUNT};
use u8g2_fonts::types::HorizontalAlignment;

use super::*;
use crate::DisplayData;

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Rows the MPPT table spends on its two heading lines: the group row naming the
/// two sides of the converter, and the row of units under it.
const MPPT_HEADING_ROWS: i32 = 2;
/// The most MPPTs that fit. Sixteen straps exist, but the panel holds twelve rows
/// under the headings, so a fully populated bus would lose the last four. The boat
/// carries eleven; `LAYOUT` is checked against this so adding a twelfth is a build
/// failure here rather than a unit silently missing from the screen.
const MPPT_ROWS: usize = (ROWS - MPPT_HEADING_ROWS) as usize;

const _: () = assert!(
    mppt_layout::LAYOUT.len() <= MPPT_ROWS,
    "more MPPTs on the boat than the information screen has rows"
);

/// Column widths of the MPPT table, in the order they are drawn.
///
/// Set by the widest reading rather than the heading, except where noted: a
/// current can go negative on a unit pushing back, so the field holds "-99.9".
///
/// The name column is set by the *heading* "MPPT" rather than by "R0" under it:
/// the group row has to start somewhere, and widening the narrowest column is
/// cheaper than letting a heading overhang the volts beside it.
const MPPT_NAME_W: i32 = 42;
const MPPT_V_W: i32 = 44;
const MPPT_I_W: i32 = 40;
/// Power is drawn whole, so four digits is the field. A five-character reading
/// would overhang 2px leftward into the 7px gap, which is why values here are
/// right-aligned and the gap is not decoration.
const MPPT_P_W: i32 = 44;
/// Set by the sub-headings "Board" and "Sink", both wider than the two- or
/// three-digit temperatures under them.
const MPPT_BOARD_W: i32 = 45;
const MPPT_SINK_W: i32 = 32;

const MPPT_WIDTHS: [i32; 9] = [
    MPPT_NAME_W,
    MPPT_V_W,
    MPPT_I_W,
    MPPT_P_W,
    MPPT_V_W,
    MPPT_I_W,
    MPPT_P_W,
    MPPT_BOARD_W,
    MPPT_SINK_W,
];

/// Fields of the side columns: label, value, unit.
const SIDE_VALUE_W: i32 = 46;
const MID_LABEL_W: i32 = 60;
/// "rpm", the widest unit the middle column carries, plus the odd pixel that
/// makes the three blocks tile the width exactly.
const MID_UNIT_W: i32 = 32;
/// Set by the "Cooling" heading rather than by "Current" under it -- a semibold
/// title runs a little wider than the same length of regular text.
const RIGHT_LABEL_W: i32 = 56;
/// "L/min", the widest unit on the screen.
const RIGHT_UNIT_W: i32 = 45;

const fn total_w<const N: usize>(widths: [i32; N]) -> i32 {
    let mut sum = FIELD_GAP * (N as i32 - 1);
    let mut i = 0;
    while i < N {
        sum += widths[i];
        i += 1;
    }
    sum
}

const MID_WIDTHS: [i32; 3] = [MID_LABEL_W, SIDE_VALUE_W, MID_UNIT_W];
const RIGHT_WIDTHS: [i32; 3] = [RIGHT_LABEL_W, SIDE_VALUE_W, RIGHT_UNIT_W];

const W_MPPT: i32 = total_w(MPPT_WIDTHS);
const W_MID: i32 = total_w(MID_WIDTHS);
const W_RIGHT: i32 = total_w(RIGHT_WIDTHS);
const GUTTER: i32 = (DISPLAY_WIDTH as i32 - (W_MPPT + W_MID + W_RIGHT)) / 2;

// The three blocks and two gutters must tile the width exactly, or the right
// column stops being flush with the screen edge.
const _: () = assert!(W_MPPT + W_MID + W_RIGHT + 2 * GUTTER == DISPLAY_WIDTH as i32);
const _: () = assert!(
    GUTTER > FIELD_GAP,
    "gutters must read wider than field gaps"
);

const BLOCKS: [Cell; 5] = SCREEN.cols([W_MPPT, GUTTER, W_MID, GUTTER, W_RIGHT]);

/// Right edge of each field in a row of `widths` laid out from `left`.
///
/// Everything in a table column is right-aligned against one of these, so the
/// digits line up down the column whatever their width. Const, so a column that
/// does not fit is a build failure.
const fn field_rights<const N: usize>(left: i32, widths: [i32; N]) -> [i32; N] {
    let mut out = [0; N];
    let mut x = left;
    let mut i = 0;
    while i < N {
        x += widths[i];
        out[i] = x;
        x += FIELD_GAP;
        i += 1;
    }
    out
}

const MPPT_R: [i32; 9] = field_rights(BLOCKS[0].x, MPPT_WIDTHS);
const MID_R: [i32; 3] = field_rights(BLOCKS[2].x, MID_WIDTHS);
const RIGHT_R: [i32; 3] = field_rights(BLOCKS[4].x, RIGHT_WIDTHS);

/// Left edge of a field, from its right edge and width. Labels and units are
/// left-aligned; only values hang off the right.
const fn left_of(right: i32, width: i32) -> i32 {
    right - width
}

// ---------------------------------------------------------------------------
// The side columns
// ---------------------------------------------------------------------------

/// One reading in a side column: what it is called, what it is measured in, and
/// how precisely it is worth reading.
struct Line {
    label: &'static str,
    /// Empty where the quantity genuinely has no unit -- the height sensors send
    /// a raw count whose scaling is still undecided, and inventing "mm" for it
    /// would be a claim the bus does not make.
    unit: &'static str,
    decimals: usize,
}

const fn line(label: &'static str, unit: &'static str, decimals: usize) -> Line {
    Line {
        label,
        unit,
        decimals,
    }
}

/// A titled group of readings, starting at a screen row.
///
/// Rows are given rather than flowed, so a section gaining a line cannot silently
/// push the one below it off the bottom of the panel -- the assertions below
/// catch it instead.
struct Section {
    title: &'static str,
    first_row: i32,
    lines: &'static [Line],
}

const fn section(title: &'static str, first_row: i32, lines: &'static [Line]) -> Section {
    Section {
        title,
        first_row,
        lines,
    }
}

/// Electrical RPM per mechanical RPM. The motor has ten pole pairs, and the VESC
/// counts in electrical revolutions -- so the number on the bus is ten times the
/// one the shaft is actually turning at, which is not what anybody wants to read.
const MOTOR_POLE_PAIRS: f32 = 10.0;

const MOTOR: [Line; 5] = [
    line("RPM", "rpm", 0),
    line("Duty", "%", 1),
    line("Current", "A", 1),
    line("FET", "\u{b0}C", 1),
    line("Motor", "\u{b0}C", 1),
];

const HEIGHT: [Line; 3] = [
    line("Left", "", 0),
    line("Right", "", 0),
    // Nothing on the bus carries the estimator's height: the flight controller
    // publishes its tuning parameters on 0x260-0x264 and its speed and course on
    // 0x201, and no frame reports the fused ride height. The row is here because
    // it belongs here, and it reads as dashes until something sends one.
    line("EKF", "", 0),
];

const THROTTLE: [Line; 1] = [line("Position", "%", 1)];

const COOLING: [Line; 3] = [
    line("In", "\u{b0}C", 1),
    line("Out", "\u{b0}C", 1),
    line("Flow", "L/min", 2),
];

const BATTERY: [Line; 5] = [
    line("Pack", "V", 1),
    line("Current", "A", 1),
    line("Charge", "%", 0),
    // Three decimals: what matters about the weakest cell is millivolts, and at
    // one decimal every cell in a healthy pack reads the same number.
    line("Lowest", "V", 3),
    line("Spread", "mV", 0),
];

const BOAT: [Line; 1] = [line("Speed", "km/h", 1)];

const MID_SECTIONS: [Section; 3] = [
    section("Motor", 0, &MOTOR),
    section("Height", 7, &HEIGHT),
    section("Throttle", 12, &THROTTLE),
];

const RIGHT_SECTIONS: [Section; 3] = [
    section("Cooling", 0, &COOLING),
    section("Battery", 5, &BATTERY),
    section("Boat", 12, &BOAT),
];

/// Last row a column of sections writes on.
const fn lowest_row<const N: usize>(sections: &[Section; N]) -> i32 {
    let mut lowest = 0;
    let mut i = 0;
    while i < N {
        let last = sections[i].first_row + sections[i].lines.len() as i32;
        if last > lowest {
            lowest = last;
        }
        i += 1;
    }
    lowest
}

/// Whether every section starts below the one before it ends, so no two overlap.
const fn sections_are_ordered<const N: usize>(sections: &[Section; N]) -> bool {
    let mut i = 1;
    while i < N {
        let previous_end = sections[i - 1].first_row + sections[i - 1].lines.len() as i32;
        if sections[i].first_row <= previous_end {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = assert!(lowest_row(&MID_SECTIONS) < ROWS, "middle column overruns");
const _: () = assert!(lowest_row(&RIGHT_SECTIONS) < ROWS, "right column overruns");
// A section must be separated from the one above by at least a blank row, or the
// titles stop reading as titles.
const _: () = assert!(sections_are_ordered(&MID_SECTIONS));
const _: () = assert!(sections_are_ordered(&RIGHT_SECTIONS));

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The placeholder for an absent value, matching the shape the value would have
/// had. Same width class as the digits it stands in for, so a column does not
/// shuffle as units come and go on the bus.
const fn dashes(decimals: usize) -> &'static str {
    match decimals {
        0 => "--",
        1 => "--.-",
        2 => "--.--",
        _ => "--.---",
    }
}

/// One right-aligned number, or dashes.
fn draw_value<D, C>(
    display: &mut D,
    buf: &mut String<16>,
    right: i32,
    y: i32,
    value: Option<f32>,
    decimals: usize,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    draw_text(
        display,
        &FONT_TINY,
        HorizontalAlignment::Right,
        right,
        y,
        fmt_f32(buf, value, decimals, dashes(decimals)),
    )
}

/// A titled group of readings in a side column. `values` is in the section's own
/// line order; the caller builds it, because only the caller knows where each
/// number comes from.
fn draw_section<D, C>(
    display: &mut D,
    buf: &mut String<16>,
    section: &Section,
    rights: [i32; 3],
    widths: [i32; 3],
    values: &[Option<f32>],
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let label_x = left_of(rights[0], widths[0]);
    let unit_x = left_of(rights[2], widths[2]);

    draw_text(
        display,
        &FONT_HEADING,
        HorizontalAlignment::Left,
        label_x,
        row_y(section.first_row),
        section.title,
    )?;

    for (index, line) in section.lines.iter().enumerate() {
        let y = row_y(section.first_row + 1 + index as i32);
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Left,
            label_x,
            y,
            line.label,
        )?;
        draw_value(
            display,
            buf,
            rights[1],
            y,
            values.get(index).copied().flatten(),
            line.decimals,
        )?;
        if !line.unit.is_empty() {
            draw_text(
                display,
                &FONT_TINY,
                HorizontalAlignment::Left,
                unit_x,
                y,
                line.unit,
            )?;
        }
    }
    Ok(())
}

/// A group heading centred over the fields it covers, `first..=last` in
/// [`MPPT_R`] order.
fn draw_group<D, C>(display: &mut D, first: usize, last: usize, text: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let left = left_of(MPPT_R[first], MPPT_WIDTHS[first]);
    draw_text(
        display,
        &FONT_HEADING,
        HorizontalAlignment::Center,
        (left + MPPT_R[last]) / 2,
        row_y(0),
        text,
    )
}

/// The MPPT table: every unit currently answering, in CAN address order.
fn draw_mppt_table<D, C>(
    display: &mut D,
    buf: &mut String<16>,
    data: &DisplayData,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    // Group row: the name column, the two sides of the converter, and the heat.
    draw_text(
        display,
        &FONT_HEADING,
        HorizontalAlignment::Left,
        BLOCKS[0].x,
        row_y(0),
        "MPPT",
    )?;
    draw_group(display, 1, 3, "Input")?;
    draw_group(display, 4, 6, "Output")?;
    draw_group(display, 7, 8, "Temp \u{b0}C")?;

    // Unit row. The temperatures take their names here instead, their unit having
    // been said once in the group above.
    for (index, text) in [
        (1, "V"),
        (2, "A"),
        (3, "W"),
        (4, "V"),
        (5, "A"),
        (6, "W"),
        (7, "Board"),
        (8, "Sink"),
    ] {
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Right,
            MPPT_R[index],
            row_y(1),
            text,
        )?;
    }

    // One row per unit that is answering, filled from the top: an MPPT that drops
    // off the bus closes its gap rather than leaving a row of dashes behind.
    let live = (0..GAN_STRAP_COUNT as u8).filter(|&strap| {
        let index = usize::from(strap);
        data.mppt_power[index].get().is_some() || data.mppt_heat[index].get().is_some()
    });

    for (row, strap) in live.take(MPPT_ROWS).enumerate() {
        let y = row_y(MPPT_HEADING_ROWS + row as i32);
        let index = usize::from(strap);

        let (side, position) = gan_side_and_position(strap);
        buf.clear();
        write!(
            buf,
            "{}{position}",
            match side {
                Side::Front => 'F',
                Side::Rear => 'R',
            }
        )
        .unwrap();
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Left,
            BLOCKS[0].x,
            y,
            buf.as_str(),
        )?;

        let power = data.mppt_power[index].get().copied();
        for (field, value, decimals) in [
            (1, power.map(|p| p.input_voltage), 1),
            (2, power.map(|p| p.input_current), 1),
            (3, power.map(|p| p.input_power()), 0),
            (4, power.map(|p| p.output_voltage), 1),
            (5, power.map(|p| p.output_current), 1),
            (6, power.map(|p| p.output_power()), 0),
        ] {
            draw_value(display, buf, MPPT_R[field], y, value, decimals)?;
        }

        let heat = data.mppt_heat[index].get().copied();
        for (field, value) in [(7, heat.map(|h| h.board)), (8, heat.map(|h| h.heat_sink))] {
            draw_value(display, buf, MPPT_R[field], y, value.map(f32::from), 0)?;
        }
    }
    Ok(())
}

/// The weakest cell and the spread across the pack, in volts and millivolts.
///
/// Only cells that are currently reporting count. A pack that has gone quiet
/// gives `None` for both rather than a spread of zero, which is what an empty
/// min-and-max would otherwise produce and would read as a perfectly balanced
/// pack instead of as no pack at all.
fn cell_extremes(data: &DisplayData) -> (Option<f32>, Option<f32>) {
    let mut lowest: Option<f32> = None;
    let mut highest: Option<f32> = None;
    for cell in &data.battery_cell_voltages {
        let Some(&v) = cell.get() else {
            continue;
        };
        lowest = Some(lowest.map_or(v, |l: f32| l.min(v)));
        highest = Some(highest.map_or(v, |h: f32| h.max(v)));
    }
    (lowest, lowest.zip(highest).map(|(l, h)| (h - l) * 1000.0))
}

pub fn draw_information<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    display.clear(BinaryColor::On.into())?;
    let mut buf: String<16> = String::new();

    draw_mppt_table(display, &mut buf, data)?;

    let (lowest_cell, cell_spread) = cell_extremes(data);

    let mid_values: [&[Option<f32>]; 3] = [
        &[
            // Mechanical, not the electrical count the VESC sends.
            data.motor_rpm
                .get()
                .map(|&erpm| erpm as f32 / MOTOR_POLE_PAIRS),
            data.motor_duty_cycle.get().copied(),
            data.motor_current.get().copied(),
            data.motor_fet_temperature.get().copied(),
            // The standalone NTC node, not the VESC's own broken reading.
            data.motor_ntc_temperature.get().copied().flatten(),
        ],
        &[
            data.height_sensor_front_left.get().map(|&v| f32::from(v)),
            data.height_sensor_front_right.get().map(|&v| f32::from(v)),
            None,
        ],
        &[data.throttle_value.get().copied()],
    ];
    for (section, values) in MID_SECTIONS.iter().zip(mid_values) {
        draw_section(display, &mut buf, section, MID_R, MID_WIDTHS, values)?;
    }

    let right_values: [&[Option<f32>]; 3] = [
        &[
            data.water_temperature_in.get().copied().flatten(),
            data.water_temperature_out.get().copied().flatten(),
            // mL/min on the bus, litres on the screen: the loop runs at a couple
            // of litres a minute, so millilitres is four digits of false precision.
            data.water_flow_in.get().map(|&ml| f32::from(ml) / 1000.0),
        ],
        &[
            data.battery_voltage.get().copied(),
            data.battery_current_pack.get().copied(),
            data.battery_state_of_charge.get().copied(),
            lowest_cell,
            cell_spread,
        ],
        &[data.speed_kmh.get().copied()],
    ];
    for (section, values) in RIGHT_SECTIONS.iter().zip(right_values) {
        draw_section(display, &mut buf, section, RIGHT_R, RIGHT_WIDTHS, values)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every heading and label has to fit the column it sits in, or it overhangs
    /// the field beside it. Values are right-aligned and may overhang leftward
    /// into a gap; text that is left-aligned has nowhere to go.
    #[test]
    fn headings_and_labels_fit_their_columns() {
        // The MPPT group headings, each against the fields it spans.
        for (first, last, text) in [(1, 3, "Input"), (4, 6, "Output"), (7, 8, "Temp \u{b0}C")] {
            let span = MPPT_R[last] - left_of(MPPT_R[first], MPPT_WIDTHS[first]);
            let w = width(&FONT_HEADING, text);
            assert!(w <= span, "{text:?} is {w}px over a {span}px group");
        }
        for (index, text) in [(7, "Board"), (8, "Sink")] {
            let w = width(&FONT_TINY, text);
            assert!(
                w <= MPPT_WIDTHS[index],
                "{text:?} is {w}px in a {}px column",
                MPPT_WIDTHS[index]
            );
        }
        let mppt = width(&FONT_HEADING, "MPPT");
        assert!(mppt <= MPPT_NAME_W, "the MPPT heading is {mppt}px");

        // Section titles and row labels against their own column's label field.
        for (sections, widths) in [(&MID_SECTIONS, MID_WIDTHS), (&RIGHT_SECTIONS, RIGHT_WIDTHS)] {
            for section in sections {
                let w = width(&FONT_HEADING, section.title);
                assert!(
                    w <= widths[0],
                    "title {:?} is {w}px in a {}px column",
                    section.title,
                    widths[0]
                );
                for line in section.lines {
                    let w = width(&FONT_TINY, line.label);
                    assert!(
                        w <= widths[0],
                        "label {:?} is {w}px in a {}px column",
                        line.label,
                        widths[0]
                    );
                    let w = width(&FONT_TINY, line.unit);
                    assert!(
                        w <= widths[2],
                        "unit {:?} is {w}px in a {}px column",
                        line.unit,
                        widths[2]
                    );
                }
            }
        }
    }

    /// The widest reading each field can hold, against the field. A value that
    /// overruns here would print over its neighbour on the panel.
    #[test]
    fn widest_values_fit_their_fields() {
        for (index, text) in [
            (1, "-99.9"),
            (2, "-99.9"),
            (3, "-999"),
            (4, "-99.9"),
            (5, "-99.9"),
            (6, "-999"),
            (7, "-99"),
            (8, "-99"),
        ] {
            let w = width(&FONT_TINY, text);
            assert!(
                w <= MPPT_WIDTHS[index],
                "{text:?} is {w}px in a {}px column",
                MPPT_WIDTHS[index]
            );
        }
        // The side columns share one value field, so one worst case covers both:
        // a four-digit RPM and a three-decimal cell voltage.
        for text in ["-9999", "3.456", "100.0"] {
            let w = width(&FONT_TINY, text);
            assert!(w <= SIDE_VALUE_W, "{text:?} is {w}px in {SIDE_VALUE_W}px");
        }
    }

    /// Dashes must be no wider than the digits they replace, or a column shifts as
    /// a unit drops off the bus.
    #[test]
    fn dashes_are_no_wider_than_their_values() {
        for (decimals, digits) in [(0, "-999"), (1, "-99.9"), (2, "99.99"), (3, "3.456")] {
            let dash = width(&FONT_TINY, dashes(decimals));
            let value = width(&FONT_TINY, digits);
            assert!(
                dash <= value,
                "{decimals} decimals: {dash}px of dashes vs {value}px"
            );
        }
    }

    /// The bottom row's ink has to stay on the panel.
    #[test]
    fn the_last_row_is_not_clipped() {
        let bottom = row_y(ROWS - 1) + INK_TOP + INK_H;
        assert!(
            bottom <= DISPLAY_HEIGHT as i32,
            "row {} reaches y={bottom}",
            ROWS - 1
        );
    }

    /// The spread is the pack's own range, and a silent pack has no spread rather
    /// than a perfect one.
    #[test]
    fn cell_extremes_need_a_reporting_pack() {
        let mut data = DisplayData::default();
        assert_eq!(cell_extremes(&data), (None, None));

        data.battery_cell_voltages[0].update(3.950);
        data.battery_cell_voltages[3].update(3.902);
        data.battery_cell_voltages[7].update(3.988);
        let (lowest, spread) = cell_extremes(&data);
        assert_eq!(lowest, Some(3.902));
        let spread = spread.expect("a spread");
        assert!((spread - 86.0).abs() < 0.1, "spread was {spread} mV");
    }
}
