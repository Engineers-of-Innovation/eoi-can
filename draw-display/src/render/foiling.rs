//! The foiling screen: trim and tuning parameters, four tables wide.
//!
//! Geometry only -- the `Cell` model, the fonts and every widget come from
//! [`super`], exactly as [`super::dashboard`] uses them.
//!
//! Four regions tile the full width: the axis rate loops on the left with a
//! column per axis, the height and rear loops next, then turn/mode/global, then
//! the config slots hard against the right edge. The status line sits in the
//! bottom-right corner, off the row grid.
//!
//! The block is parked against the top edge -- the heading row's ink starts on
//! y=0 -- because the panel is white past its active area, so text on the edge
//! reads as a margin rather than as clipping. What that frees pays for the gap
//! above the `Rear` and `Mode` headings and for the status line's own baseline.
//!
//! Three ideas do the heavy lifting:
//!
//! - **Rows are the shared suffix, not the parameter name.** `RLL_RATE_P` and
//!   `PTCH_RATE_P` are one `RATE_P` row with a value per column, which is what
//!   turns 27 axis parameters into 13 rows.
//! - **An up/down pair collapses while it is symmetric.** `PTCH2SRV_RMAX_UP` and
//!   `_DN` read as one number while they agree and as `60^ 45v` when they do not.
//!   Same for the pitch envelope and the height loop's command clamps.
//! - **Hotkeys are unique across the whole screen**, so one keypress selects one
//!   cell with nothing to cycle through -- which matters on a panel that takes a
//!   second to redraw. Case carries meaning: `P` is the rate loop's P gain and
//!   `p` the height loop's.
//!
//! The display is a pure renderer. It holds no slot state, no undo history and
//! no cursor of its own; values, slot times, the selected cell and the last edit
//! all arrive over CAN and are drawn as received.

use core::fmt::Write;

use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use heapless::String;
use u8g2_fonts::types::{FontColor, HorizontalAlignment, VerticalPosition};

use super::*;
use crate::{
    DisplayData, FoilColumn, FoilConfigAction, FoilEdit, FoilEvent, FoilLimit, FoilSlotEvent,
    FoilingData, Latched, Reading,
};

/// One row of a table. `decimals` belongs to the parameter, not the value, so a
/// gain renders to the same precision whatever it currently reads.
struct Row {
    hotkey: &'static str,
    label: &'static str,
    decimals: usize,
    /// Drawn after the value, empty for the many parameters that are pure gains
    /// or ratios. A blank slot rather than a shifted value: the digits have to go
    /// on lining up down the column whether their neighbours carry a unit or not.
    unit: &'static str,
}

const fn row(
    hotkey: &'static str,
    label: &'static str,
    decimals: usize,
    unit: &'static str,
) -> Row {
    Row {
        hotkey,
        label,
        decimals,
        unit,
    }
}

/// A group of rows within a column, with the screen row it starts on.
struct Block {
    /// Shown in the status line, so it reads "PITCH RATE_P increased ...".
    name: &'static str,
    rows: &'static [Row],
    /// Screen row of this block's first parameter.
    first_row: i32,
    /// Whether a heading is drawn on the row above `first_row`.
    heading: bool,
}

/// Extra space above a stacked column's second and later blocks.
///
/// On the row grid alone a `Rear` or `Mode` heading sits one pitch below the last
/// row of the block above, which reads as another entry rather than as a new
/// table. This offsets everything from the heading down, so the gap lands where
/// the eye needs it. Paid for by [`BLOCK_TOP`] moving the whole grid up.
const SECTION_GAP: i32 = 6;

impl Block {
    /// Vertical offset applied to this block, heading included.
    const fn y_offset(&self) -> i32 {
        if self.first_row > 1 {
            SECTION_GAP
        } else {
            0
        }
    }
}

/// The axis rate loops. `RMAX` and `LIMIT` each collapse an up/down pair.
///
/// Labels keep the `RATE_` prefix the real parameters carry. Dropping it frees
/// 14px, but leaves rows reading `P  P` and a status line saying "PITCH P
/// increased", so the width comes out of `FIELD_GAP` instead.
const AXIS: [Row; 13] = [
    row("P", "RATE_P", 2, ""),
    row("I", "RATE_I", 2, ""),
    row("D", "RATE_D", 3, ""),
    row("F", "RATE_FF", 2, ""),
    row("M", "RATE_IMAX", 1, ""),
    row("C", "TCONST", 2, "s"),
    row("R", "RMAX", 0, "\u{b0}/s"),
    row("L", "LIMIT", 0, "\u{b0}"),
    row("T", "FLT_T", 0, "Hz"),
    row("E", "FLT_E", 0, "Hz"),
    row("G", "FLT_D", 0, "Hz"),
    row("S", "SMAX", 0, ""),
    row("X", "RLL>PTCH", 2, ""),
];

/// The ride-height loop. `CMD` collapses `HYD_CMDMAX`/`HYD_CMDMIN`.
///
/// Lowercase mirrors [`AXIS`] where the concept is the same: `p`/`d` against
/// `P`/`D`. `k` stands in for the I gain because lowercase `i` is unusable beside
/// `I` and `1`, and `h` for IMAX because lowercase `m` would read as a small `M`.
const HEIGHT: [Row; 7] = [
    row("p", "KP", 0, ""),
    row("k", "KI", 0, ""),
    row("d", "KD", 0, ""),
    row("h", "IMAX", 0, ""),
    row("t", "TARGET", 2, "m"),
    row("g", "CMD", 1, "\u{b0}"),
    row("b", "ARM", 2, "m"),
];

/// The rear foil: artificial tailplane, decalage and speed schedule.
///
/// Four rows, so the heading fits: the block runs 9..12 and row 13 belongs to the
/// status line everywhere but the axis column. `RTKI`, the rear trim's I gain,
/// held row 9 here until 2026-08-25 and cost the heading to do it -- it is off
/// the grid now, and `HYD_RTKI` stays at 0 over MAVLink.
const REAR: [Row; 4] = [
    row("K", "RKP", 2, ""),
    row("W", "RSCALE", 2, ""),
    row("Y", "RSCHED", 0, ""),
    row("V", "FRNTFF", 2, ""),
];

/// Coordinated-turn banking. `ENABLE` stays keyed as the in-flight kill switch.
const TURN: [Row; 6] = [
    row("N", "ENABLE", 0, ""),
    row("U", "ON", 0, "%"),
    row("A", "FULL", 0, "%"),
    row("Z", "MAX", 1, "\u{b0}"),
    row("H", "RATE", 1, "\u{b0}/s"),
    row("J", "REV", 0, ""),
];

/// Operating mode and the live test demands: `SCR_USER1..4` under names that mean
/// something, since the real ones do not.
const MODE: [Row; 4] = [
    row("y", "MODE", 0, ""),
    row("q", "TEST_P", 1, "\u{b0}"),
    row("f", "TEST_R", 1, "\u{b0}"),
    row("B", "JOG", 0, "\u{b5}s"),
];

/// Gain scaling, which rescales both axes at once and so belongs to neither.
/// Drawn without a heading: one row does not earn a line of its own, and the
/// row it would have taken is the status line's.
const GLOBAL: [Row; 1] = [row("Q", "SPEED", 1, "m/s")];

const MID_BLOCKS: [Block; 2] = [
    Block {
        name: "HEIGHT",
        rows: &HEIGHT,
        first_row: 1,
        heading: true,
    },
    Block {
        name: "REAR",
        rows: &REAR,
        first_row: 9,
        heading: true,
    },
];

const RIGHT_BLOCKS: [Block; 3] = [
    Block {
        name: "TURN",
        rows: &TURN,
        first_row: 1,
        heading: true,
    },
    Block {
        name: "MODE",
        rows: &MODE,
        first_row: 8,
        heading: true,
    },
    Block {
        name: "GLOBAL",
        rows: &GLOBAL,
        first_row: 12,
        heading: false,
    },
];

/// Nine stores, plus undo and factory reset, keyed by the number row.
const SLOT_COUNT: usize = 9;

/// The keys under the slots. `store` is not a slot: it commits the live tune to
/// the flight controller's flash, so what is on the boat now survives a power
/// cycle -- the nine slots above it are RAM and do not.
const SLOT_ACTIONS: [(&str, &str); 3] = [("~", "undo"), ("0", "factory"), ("]", "store")];

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Padding around a value's cap band when its cell is inverted.
const CURSOR_PAD: i32 = 2;
const CURSOR_H: i32 = TINY_CAP_H + 2 * CURSOR_PAD;

// The inverted cell must cover a value's ink without reaching the rows either
// side. Digits leave the descender space empty, which is where the slack is.
const _: () = assert!(CURSOR_H <= ROW_H);
const _: () = assert!(CURSOR_PAD + CURSOR_PAD + TINY_CAP_H <= ROW_H);

/// Column widths, each the sum of its fields, and the gutters chosen so the four
/// tile the full width exactly.
const HOTKEY_W: i32 = 17;
const AXIS_LABEL_W: i32 = 94;
/// Sized for an ordinary value, **not** for a diverged pair.
///
/// A diverged pair is 92px -- an arrow alone is 14px -- and giving the column
/// that width would leave 40px of dead space on all eleven rows that never diverge,
/// which read as a value adrift from its label. Instead the pair overflows
/// leftward into the gap its own label leaves: only `RMAX` and `LIMIT` diverge,
/// and both have short labels. `diverged_values_clear_their_labels` checks it.
const AXIS_PITCH_W: i32 = 52;
/// Roll has no up/down pairs -- `RLL2SRV_RMAX` and `ROLL_LIMIT_DEG` are single
/// values -- so this column never holds an arrow pair.
const AXIS_ROLL_W: i32 = 46;
/// Widest unit the axis table carries, `°/s` for `RMAX`. One slot serves both
/// columns: the unit belongs to the row, not the axis, so `RMAX` is degrees per
/// second whichever side you read.
const AXIS_UNIT_W: i32 = 22;
const W_AXIS: i32 =
    HOTKEY_W + AXIS_LABEL_W + AXIS_PITCH_W + AXIS_ROLL_W + AXIS_UNIT_W + 4 * FIELD_GAP;

const MID_LABEL_W: i32 = 68;
/// Sized for an ordinary value. `CMD` is the one diverged pair here and, like the
/// axis pairs, overflows left past its short label -- see [`AXIS_PITCH_W`].
const MID_VALUE_W: i32 = 52;
/// `m` for the ride height and the reference arm; `°` for the command clamps.
///
/// The rear speed schedule's real unit is `°·m²/s²`, which measures 52px and
/// would set this slot for all eleven rows at a cost of 13px off each gutter. Not
/// taken: one row's unit is not worth a third of the space between the tables,
/// and a unit only legible to someone who already knows it earns little.
const MID_UNIT_W: i32 = 15;
const W_MID: i32 = HOTKEY_W + MID_LABEL_W + MID_VALUE_W + MID_UNIT_W + 3 * FIELD_GAP;

const RIGHT_LABEL_W: i32 = 66;
const RIGHT_VALUE_W: i32 = 46;
/// Widest on the screen, `m/s` for the gain-scaling reference speed.
const RIGHT_UNIT_W: i32 = 30;
const W_RIGHT: i32 = HOTKEY_W + RIGHT_LABEL_W + RIGHT_VALUE_W + RIGHT_UNIT_W + 3 * FIELD_GAP;

const SLOT_LABEL_W: i32 = 54;
const SLOT_KEY_W: i32 = 12;
const SLOT_GAP: i32 = 6;
const W_SLOTS: i32 = SLOT_LABEL_W + SLOT_GAP + SLOT_KEY_W;

const GUTTER: i32 = (DISPLAY_WIDTH as i32 - (W_AXIS + W_MID + W_RIGHT + W_SLOTS)) / 3;

// The four columns plus three gutters must tile the width exactly, or the config
// slots stop being flush with the right edge.
const _: () = assert!(W_AXIS + W_MID + W_RIGHT + W_SLOTS + 3 * GUTTER == DISPLAY_WIDTH as i32);
const _: () = assert!(
    GUTTER > FIELD_GAP,
    "gutters must read wider than field gaps"
);
// The inverted cell overhangs its column by `CURSOR_PAD`, which the gap to the
// next field has to cover.
const _: () = assert!(CURSOR_PAD < FIELD_GAP && CURSOR_PAD < SLOT_GAP);

const COLS: [Cell; 7] = SCREEN.cols([W_AXIS, GUTTER, W_MID, GUTTER, W_RIGHT, GUTTER, W_SLOTS]);

const AXIS_HOTKEY_X: i32 = COLS[0].x;
const AXIS_LABEL_X: i32 = AXIS_HOTKEY_X + HOTKEY_W + FIELD_GAP;
const AXIS_PITCH_R: i32 = AXIS_LABEL_X + AXIS_LABEL_W + FIELD_GAP + AXIS_PITCH_W;
/// Space a right-aligned pitch value may grow into before it reaches the label
/// column: the pitch column plus the gap plus the label column itself. Only the
/// test that enforces the overflow reads it, but it is the rule being enforced.
#[cfg(test)]
const AXIS_PITCH_SPAN: i32 = AXIS_LABEL_W + FIELD_GAP + AXIS_PITCH_W;
const AXIS_ROLL_R: i32 = AXIS_PITCH_R + FIELD_GAP + AXIS_ROLL_W;
const AXIS_UNIT_X: i32 = AXIS_ROLL_R + FIELD_GAP;

const MID_HOTKEY_X: i32 = COLS[2].x;
const MID_LABEL_X: i32 = MID_HOTKEY_X + HOTKEY_W + FIELD_GAP;
const MID_VALUE_R: i32 = MID_LABEL_X + MID_LABEL_W + FIELD_GAP + MID_VALUE_W;
/// As `AXIS_PITCH_SPAN`, for the height and rear column.
#[cfg(test)]
const MID_VALUE_SPAN: i32 = MID_LABEL_W + FIELD_GAP + MID_VALUE_W;

const RIGHT_HOTKEY_X: i32 = COLS[4].x;
const RIGHT_LABEL_X: i32 = RIGHT_HOTKEY_X + HOTKEY_W + FIELD_GAP;
const RIGHT_VALUE_R: i32 = RIGHT_LABEL_X + RIGHT_LABEL_W + FIELD_GAP + RIGHT_VALUE_W;

/// The slots mirror the parameter tables: their key sits at the far edge and the
/// label grows inward, so the two hotkey strips are the two screen edges.
const SLOT_KEY_R: i32 = COLS[6].right();
const SLOT_LABEL_R: i32 = SLOT_KEY_R - SLOT_KEY_W - SLOT_GAP;

/// The status line sits in the bottom-right corner, off the row grid.
///
/// Its own baseline rather than row 13's: the corner is the one place a sentence
/// can be as long as it needs without a column to answer to, and hugging the edge
/// keeps it clearly separate from the tables instead of reading as another row.
/// Right-aligned on the screen edge, like the config keys above it.
const STATUS_R: i32 = DISPLAY_WIDTH as i32;
/// Ink bottom on the last pixel row, mirroring what `BLOCK_TOP` does at the top.
const STATUS_Y: i32 = DISPLAY_HEIGHT as i32 - 1 - INK_H - INK_TOP;
/// It overlaps row 13 vertically, so it has to stay clear of that row horizontally.
/// Only the axis table reaches row 13.
#[cfg(test)]
const STATUS_W: i32 = DISPLAY_WIDTH as i32 - COLS[0].right() - GUTTER;

// Every table has to fit inside the rows, headings included.
// The axis table starts on row 1, so its last row is the bottom one.
const _: () = assert!(AXIS.len() as i32 == ROWS - 1);
const _: () = assert!(MID_BLOCKS[1].first_row + (REAR.len() as i32) < ROWS);
const _: () = assert!(RIGHT_BLOCKS[2].first_row + (GLOBAL.len() as i32) < ROWS);
const _: () = assert!((SLOT_COUNT + SLOT_ACTIONS.len()) as i32 + 1 < ROWS);
// The slot column is the status line's own side of the screen, so its last row --
// the actions, which sit below the nine slots -- has to stay above that ink.
const LOWEST_SLOT_INK: i32 = row_y((SLOT_COUNT + SLOT_ACTIONS.len()) as i32) + INK_TOP + INK_H;
const _: () = assert!(LOWEST_SLOT_INK < STATUS_Y + INK_TOP);
// The lowest stacked row, pushed down by SECTION_GAP, must not reach the status
// line's ink -- they share the right-hand side of the screen.
const LOWEST_STACKED_INK: i32 = row_y(ROWS - 2) + SECTION_GAP + INK_TOP + INK_H;
const _: () = assert!(LOWEST_STACKED_INK < STATUS_Y + INK_TOP);
// And nothing runs off the bottom.
const _: () = assert!(STATUS_Y + INK_TOP + INK_H < DISPLAY_HEIGHT as i32);

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

pub fn draw_foiling<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    display.clear(BinaryColor::On.into())?;
    let foil = &data.foiling;
    let mut buf: String<16> = String::new();

    // Axis table: two value columns, headed by the axis names.
    for (right, name) in [(AXIS_PITCH_R, "Pitch"), (AXIS_ROLL_R, "Roll")] {
        draw_text(
            display,
            &FONT_HEADING,
            HorizontalAlignment::Right,
            right,
            row_y(0),
            name,
        )?;
    }
    for (index, entry) in AXIS.iter().enumerate() {
        let y = row_y(1 + index as i32);
        key_and_label(display, AXIS_HOTKEY_X, AXIS_LABEL_X, y, entry)?;
        value(
            display,
            AXIS_PITCH_R,
            y,
            entry,
            &foil.pitch[index],
            &mut buf,
        )?;
        value(display, AXIS_ROLL_R, y, entry, &foil.roll[index], &mut buf)?;
        // One slot for both columns: the unit belongs to the row, not the axis.
        unit(display, AXIS_UNIT_X, y, entry)?;
    }

    // The two stacked columns. Each block's values come from its own array, so a
    // block gaining a row cannot silently shift the one below it.
    for (block, values) in MID_BLOCKS.iter().zip([&foil.height[..], &foil.rear[..]]) {
        draw_block(
            display,
            block,
            values,
            MID_HOTKEY_X,
            MID_LABEL_X,
            MID_VALUE_R,
            &mut buf,
        )?;
    }
    for (block, values) in
        RIGHT_BLOCKS
            .iter()
            .zip([&foil.turn[..], &foil.mode[..], &foil.global[..]])
    {
        draw_block(
            display,
            block,
            values,
            RIGHT_HOTKEY_X,
            RIGHT_LABEL_X,
            RIGHT_VALUE_R,
            &mut buf,
        )?;
    }

    draw_slots(display, foil, &mut buf)?;
    draw_status(display, foil)?;
    draw_cursor(display, foil, &mut buf)
}

#[allow(clippy::too_many_arguments)]
fn draw_block<D, C>(
    display: &mut D,
    block: &Block,
    values: &[Latched<Reading>],
    hotkey_x: i32,
    label_x: i32,
    value_r: i32,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let offset = block.y_offset();
    if block.heading {
        // Over the label column, not the values: these tables have one value
        // column, so the name belongs to the group rather than the numbers.
        let title = heading_title(block.name);
        draw_text(
            display,
            &FONT_HEADING,
            HorizontalAlignment::Left,
            label_x,
            row_y(block.first_row - 1) + offset,
            title.as_str(),
        )?;
    }
    for (index, entry) in block.rows.iter().enumerate() {
        let y = row_y(block.first_row + index as i32) + offset;
        key_and_label(display, hotkey_x, label_x, y, entry)?;
        value(display, value_r, y, entry, &values[index], buf)?;
        // The unit slot opens one field gap past the value column, whichever
        // column this block is in.
        unit(display, value_r + FIELD_GAP, y, entry)?;
    }
    Ok(())
}

/// The unit slot: left-aligned against the value it belongs to, and skipped
/// entirely when the parameter has none.
fn unit<D, C>(display: &mut D, x: i32, y: i32, entry: &Row) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    if entry.unit.is_empty() {
        return Ok(());
    }
    draw_text(
        display,
        &FONT_TINY,
        HorizontalAlignment::Left,
        x,
        y,
        entry.unit,
    )
}

fn key_and_label<D, C>(
    display: &mut D,
    hotkey_x: i32,
    label_x: i32,
    y: i32,
    entry: &Row,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    draw_text(
        display,
        &FONT_TINY,
        HorizontalAlignment::Left,
        hotkey_x,
        y,
        entry.hotkey,
    )?;
    draw_text(
        display,
        &FONT_TINY,
        HorizontalAlignment::Left,
        label_x,
        y,
        entry.label,
    )
}

/// Format a reading into `buf`. Absent renders as dashes, matching the
/// dashboard: a parameter the bus has stopped reporting must not keep showing
/// its last number.
fn format_reading(buf: &mut String<16>, reading: Option<&Reading>, decimals: usize) {
    buf.clear();
    match reading {
        Some(Reading::One(v)) => {
            write!(buf, "{v:.decimals$}").ok();
        }
        Some(Reading::UpDown(up, down)) => {
            // The down value is drawn as a magnitude: the pitch envelope's
            // minimum is negative on the bus, and a down arrow beside a minus
            // sign reads as a double negative.
            let magnitude = if *down < 0.0 { -*down } else { *down };
            if (up - magnitude).abs() < f32::EPSILON {
                // Symmetric, so it reads as one number. Collapsing here rather
                // than at the sender means whoever fills these in can always
                // store both halves and never has to decide which form to use.
                write!(buf, "{up:.decimals$}").ok();
            } else {
                write!(
                    buf,
                    "{up:.decimals$}\u{2191} {magnitude:.decimals$}\u{2193}"
                )
                .ok();
            }
        }
        None => {
            buf.push_str("--").ok();
        }
    }
}

fn value<D, C>(
    display: &mut D,
    right: i32,
    y: i32,
    entry: &Row,
    reading: &Latched<Reading>,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    format_reading(buf, reading.get(), entry.decimals);
    draw_text(
        display,
        &FONT_TINY,
        HorizontalAlignment::Right,
        right,
        y,
        buf.as_str(),
    )
}

/// A slot's label: never stored, stored without a fix, or the time it was taken.
fn format_slot(buf: &mut String<16>, slot: Option<&Option<(u8, u8)>>) {
    buf.clear();
    match slot {
        Some(Some((hours, minutes))) => {
            write!(buf, "{hours:02}:{minutes:02}").ok();
        }
        // Should not arise -- nobody stores a tune without GNSS speed -- but it
        // keeps the column one shape.
        Some(None) => {
            buf.push_str("--:--").ok();
        }
        None => {
            buf.push_str("empty").ok();
        }
    }
}

/// A block's heading as it is drawn: `HEIGHT` reads as `Height`.
///
/// The name is stored in capitals because the status line says "HEIGHT CMD
/// increased ...", where it stands beside a parameter label that is capitals too.
/// A heading is a word rather than a label, so it is drawn as one.
fn heading_title(name: &str) -> String<16> {
    let mut title: String<16> = String::new();
    for (index, ch) in name.chars().enumerate() {
        let ch = if index == 0 {
            ch
        } else {
            ch.to_ascii_lowercase()
        };
        title.push(ch).ok();
    }
    title
}

fn draw_slots<D, C>(
    display: &mut D,
    foil: &FoilingData,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    draw_text(
        display,
        &FONT_HEADING,
        HorizontalAlignment::Right,
        SLOT_KEY_R,
        row_y(0),
        "Configs",
    )?;

    for (index, slot) in foil.slots.iter().enumerate() {
        let y = row_y(1 + index as i32);
        format_slot(buf, slot.get());
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Right,
            SLOT_LABEL_R,
            y,
            buf.as_str(),
        )?;
        buf.clear();
        write!(buf, "{}", index + 1).ok();
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Right,
            SLOT_KEY_R,
            y,
            buf.as_str(),
        )?;
    }

    for (offset, (key, label)) in SLOT_ACTIONS.iter().enumerate() {
        let y = row_y(1 + SLOT_COUNT as i32 + offset as i32);
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Right,
            SLOT_LABEL_R,
            y,
            label,
        )?;
        draw_text(
            display,
            &FONT_TINY,
            HorizontalAlignment::Right,
            SLOT_KEY_R,
            y,
            key,
        )?;
    }
    Ok(())
}

/// Which half of a collapsed up/down pair a parameter index is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairHalf {
    /// Not a pair: the index is the whole cell.
    Whole,
    Up,
    Down,
}

/// Where a `foil_tune.lua` parameter index lands on this screen.
///
/// The tuner addresses parameters by index and the screen is laid out by cell, so
/// something has to join the two. It lives here, beside the row tables it has to
/// agree with; `FOILING_PARAMETERS.csv` carries the same mapping outwards for the
/// datalogger, and a test checks the two agree.
///
/// Indices are `foil_tune.lua`'s `PT` table at PROTO_VERSION 9. 13-15 and 46-47
/// are unused; 37-38 and 58 are retired and must never be reused. 59 (`HYD_RTKI`)
/// is still on the flight controller's whitelist but has no cell here, so the
/// screen cannot reach it -- the index stays spoken for either way.
pub const fn cell_for_index(index: u8) -> Option<(FoilColumn, u8, PairHalf)> {
    use FoilColumn::{Mid, Pitch, Right, Roll};
    use PairHalf::{Down, Up, Whole};
    Some(match index {
        // Roll rate loop. Roll has no up/down pairs.
        1 => (Roll, 1, Whole),
        2 => (Roll, 2, Whole),
        3 => (Roll, 3, Whole),
        4 => (Roll, 4, Whole),
        5 => (Roll, 5, Whole),
        6 => (Roll, 6, Whole),
        7 => (Roll, 7, Whole),
        8 => (Roll, 8, Whole),
        9 => (Roll, 9, Whole),
        10 => (Roll, 10, Whole),
        11 => (Roll, 11, Whole),
        12 => (Roll, 12, Whole),
        // Pitch rate loop. RMAX and LIMIT are each two parameters in one cell.
        16 => (Pitch, 1, Whole),
        17 => (Pitch, 2, Whole),
        18 => (Pitch, 3, Whole),
        19 => (Pitch, 4, Whole),
        20 => (Pitch, 5, Whole),
        21 => (Pitch, 6, Whole),
        22 => (Pitch, 7, Up),
        23 => (Pitch, 7, Down),
        24 => (Pitch, 13, Whole),
        25 => (Pitch, 8, Up),
        26 => (Pitch, 8, Down),
        27 => (Pitch, 9, Whole),
        28 => (Pitch, 10, Whole),
        29 => (Pitch, 11, Whole),
        30 => (Pitch, 12, Whole),
        // Gain scaling, which belongs to neither axis.
        31 => (Right, 12, Whole),
        // Ride-height loop. The command clamps share a cell.
        32 => (Mid, 1, Whole),
        33 => (Mid, 2, Whole),
        34 => (Mid, 3, Whole),
        35 => (Mid, 4, Whole),
        36 => (Mid, 5, Whole),
        52 => (Mid, 6, Up),
        53 => (Mid, 6, Down),
        39 => (Mid, 7, Whole),
        // Rear foil. 58 and 59 are absent by design: 58 is retired, and 59
        // (HYD_RTKI) is off the screen while it stays on the FC whitelist.
        54 => (Mid, 9, Whole),
        55 => (Mid, 10, Whole),
        56 => (Mid, 11, Whole),
        57 => (Mid, 12, Whole),
        // Coordinated turn.
        40 => (Right, 1, Whole),
        41 => (Right, 2, Whole),
        42 => (Right, 3, Whole),
        43 => (Right, 4, Whole),
        44 => (Right, 5, Whole),
        45 => (Right, 6, Whole),
        // Mode and the live test demands.
        48 => (Right, 8, Whole),
        49 => (Right, 9, Whole),
        50 => (Right, 10, Whole),
        51 => (Right, 11, Whole),
        _ => return None,
    })
}

/// Look up the block and row a cell belongs to, for the cursor and status line.
fn locate(column: FoilColumn, screen_row: i32) -> Option<(&'static str, &'static Row)> {
    match column {
        FoilColumn::Pitch | FoilColumn::Roll => {
            let name = if matches!(column, FoilColumn::Pitch) {
                "PITCH"
            } else {
                "ROLL"
            };
            let index = usize::try_from(screen_row - 1).ok()?;
            AXIS.get(index).map(|entry| (name, entry))
        }
        FoilColumn::Mid | FoilColumn::Right => {
            let blocks: &'static [Block] = if matches!(column, FoilColumn::Mid) {
                &MID_BLOCKS
            } else {
                &RIGHT_BLOCKS
            };
            blocks.iter().find_map(|block| {
                let index = usize::try_from(screen_row - block.first_row).ok()?;
                block.rows.get(index).map(|entry| (block.name, entry))
            })
        }
        FoilColumn::Slot => None,
    }
}

/// The sentence the status line shows, composed here from the row tables so that
/// nothing on the bus has to carry text.
///
/// The buffer is sized in [`the_longest_status_line_fits`].
fn status_line(event: FoilEvent) -> Option<String<80>> {
    match event {
        FoilEvent::Edit(edit) => edit_line(edit),
        FoilEvent::Slot(slot) => slot_line(slot),
    }
}

/// What the last configuration-slot key did.
///
/// Whether the timestamp is the moment of storing or the age of the tune being put
/// back depends on the action, so the preposition changes with it: `stored at
/// 14:32` against `restored from 14:32`.
fn slot_line(event: FoilSlotEvent) -> Option<String<80>> {
    let mut line: String<80> = String::new();
    match event.action {
        action @ (FoilConfigAction::Stored | FoilConfigAction::Restored) => {
            let stored = matches!(action, FoilConfigAction::Stored);
            // A slot number outside the column is not drawn at all: the sentence
            // would name a config the screen does not have.
            if !(1..=SLOT_COUNT as u8).contains(&event.slot) {
                return None;
            }
            let verb = if stored { "stored" } else { "restored" };
            write!(&mut line, "config {} {verb}", event.slot).ok()?;
            if let Some((hour, minute)) = event.time {
                let preposition = if stored { "at" } else { "from" };
                write!(&mut line, " {preposition} {hour:02}:{minute:02}").ok()?;
            }
        }
        FoilConfigAction::Undone => line.push_str("last change undone").ok()?,
        FoilConfigAction::FactoryReset => line.push_str("factory tune restored").ok()?,
        // The one action that outlives a power cycle, so it says so plainly.
        FoilConfigAction::SavedToFlash => line.push_str("tune saved to flash").ok()?,
        // Nothing to say about an action this build does not know.
        FoilConfigAction::Unknown(_) => return None,
    }
    Some(line)
}

/// What the last parameter edit did.
///
/// Three wordings, because an edit can end three ways. A write that went through
/// reads as a movement; a write the flight controller clamped says so, since the
/// number that appears is the bound and not what was asked for; and a write that
/// was clamped without moving at all -- the cell was already on its limit -- has
/// no movement to report and would otherwise read as a display fault ("increased
/// from 8.00 to 8.00").
fn edit_line(edit: FoilEdit) -> Option<String<80>> {
    let (group, entry) = locate(edit.column, edit.row as i32)?;
    let decimals = entry.decimals;
    let bound = match edit.clamped {
        Some(FoilLimit::Min) => "min",
        Some(FoilLimit::Max) => "max",
        // Clamped against a bound the bus does not name. Vague on purpose: see
        // `FoilLimit::Unknown`.
        Some(FoilLimit::Unknown) => "its limit",
        None => "",
    };

    let mut line: String<80> = String::new();
    if edit.clamped.is_some() && (edit.to - edit.from).abs() <= f32::EPSILON {
        write!(
            &mut line,
            "{group} {} already at {bound} {:.decimals$}",
            entry.label, edit.to
        )
        .ok()?;
    } else {
        let verb = if edit.to >= edit.from {
            "increased"
        } else {
            "decreased"
        };
        write!(
            &mut line,
            "{group} {} {verb} from {:.decimals$} to {:.decimals$}",
            entry.label, edit.from, edit.to
        )
        .ok()?;
        if edit.clamped.is_some() {
            write!(&mut line, ", clamped at {bound}").ok()?;
        }
    }
    Some(line)
}

/// The status line: what the last edit did, in words.
///
/// Held until the next edit replaces it -- there is no timeout, because the
/// point of the line is to still be readable a while after the change.
fn draw_status<D, C>(display: &mut D, foil: &FoilingData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let Some(line) = foil.last_event.and_then(status_line) else {
        return Ok(());
    };

    draw_text(
        display,
        &FONT_TINY,
        HorizontalAlignment::Right,
        STATUS_R,
        STATUS_Y,
        line.as_str(),
    )
}

/// Invert the selected cell: fill it with ink, then redraw its text in the
/// background colour.
///
/// An outline was tried first and does not work at this size. `draw_text`
/// centres on the font's full ink box, descenders included, so a string of
/// digits sits high in its row -- its ink starts exactly where a row-height
/// outline's top edge falls, and the two touch. Inverting sidesteps the
/// alignment entirely, and it is what a selection should look like anyway.
fn draw_cursor<D, C>(
    display: &mut D,
    foil: &FoilingData,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let Some(cursor) = foil.cursor.get() else {
        return Ok(());
    };
    let row = cursor.row as i32;
    let (right, width) = match cursor.column {
        FoilColumn::Pitch => (AXIS_PITCH_R, AXIS_PITCH_W),
        FoilColumn::Roll => (AXIS_ROLL_R, AXIS_ROLL_W),
        FoilColumn::Mid => (MID_VALUE_R, MID_VALUE_W),
        FoilColumn::Right => (RIGHT_VALUE_R, RIGHT_VALUE_W),
        FoilColumn::Slot => (SLOT_LABEL_R, SLOT_LABEL_W),
    };

    // What the cell says, re-derived so the inverted copy cannot disagree with
    // the one already drawn.
    match cursor.column {
        FoilColumn::Slot => {
            let Some(index) = usize::try_from(row - 1).ok().filter(|i| *i < SLOT_COUNT) else {
                return Ok(());
            };
            format_slot(buf, foil.slots[index].get());
        }
        FoilColumn::Pitch | FoilColumn::Roll => {
            let Some((_, entry)) = locate(cursor.column, row) else {
                return Ok(());
            };
            let values = if matches!(cursor.column, FoilColumn::Pitch) {
                &foil.pitch[..]
            } else {
                &foil.roll[..]
            };
            let Some(reading) = values.get((row - 1) as usize) else {
                return Ok(());
            };
            format_reading(buf, reading.get(), entry.decimals);
        }
        FoilColumn::Mid | FoilColumn::Right => {
            let Some((reading, entry)) = stacked_cell(cursor.column, foil, row) else {
                return Ok(());
            };
            format_reading(buf, reading.get(), entry.decimals);
        }
    }

    let y = cell_y(cursor.column, row);
    // Padded on the right as well: `right` is the value's right-align anchor, so
    // an unpadded fill would end exactly on the last digit's edge. The field gap
    // to the next column absorbs it.
    Rectangle::new(
        Point::new(right - width, y + INK_TOP - CURSOR_PAD),
        Size::new((width + CURSOR_PAD) as u32, CURSOR_H as u32),
    )
    .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off.into()))
    .draw(display)?;

    // The one place this screen draws in the background colour.
    FONT_TINY
        .render_aligned(
            buf.as_str(),
            Point::new(right, y),
            VerticalPosition::Center,
            HorizontalAlignment::Right,
            FontColor::Transparent(BinaryColor::On.into()),
            display,
        )
        .map_err(map_font_err)?;
    Ok(())
}

/// Centre y of a cell, which is not `row_y` alone for the stacked columns: a
/// block below the first is pushed down by [`SECTION_GAP`], so anything placed on
/// the bare row grid lands on the row above it. `draw_block` adds the offset when
/// it draws the value; the cursor has to add the same one to invert it.
fn cell_y(column: FoilColumn, row: i32) -> i32 {
    let blocks: &'static [Block] = match column {
        FoilColumn::Mid => &MID_BLOCKS,
        FoilColumn::Right => &RIGHT_BLOCKS,
        // One table each, drawn straight off the grid.
        FoilColumn::Pitch | FoilColumn::Roll | FoilColumn::Slot => return row_y(row),
    };
    let offset = blocks
        .iter()
        .find(|block| row >= block.first_row && row < block.first_row + block.rows.len() as i32)
        .map_or(0, Block::y_offset);
    row_y(row) + offset
}

/// The array and index a stacked column's row draws from, so the inverted copy
/// reads the same value the first pass drew.
fn stacked_cell<'a>(
    column: FoilColumn,
    foil: &'a FoilingData,
    row: i32,
) -> Option<(&'a Latched<Reading>, &'static Row)> {
    let (block, index) = stacked_slot(column, row)?;
    let arrays: [&'a [Latched<Reading>]; 3] = if matches!(column, FoilColumn::Mid) {
        [&foil.height[..], &foil.rear[..], &[]]
    } else {
        [&foil.turn[..], &foil.mode[..], &foil.global[..]]
    };
    let entry = blocks_of(column)?.get(block)?.rows.get(index)?;
    Some((arrays.get(block)?.get(index)?, entry))
}

/// The block tables a stacked column is drawn from, in the order their value
/// arrays are listed.
fn blocks_of(column: FoilColumn) -> Option<&'static [Block]> {
    match column {
        FoilColumn::Mid => Some(&MID_BLOCKS),
        FoilColumn::Right => Some(&RIGHT_BLOCKS),
        FoilColumn::Pitch | FoilColumn::Roll | FoilColumn::Slot => None,
    }
}

/// The reading the screen actually draws in a cell, straight out of the arrays
/// `draw_foiling` reads. Lets a test assert that a read-back is drawn where it was
/// stored, which neither side can check alone.
#[cfg(test)]
pub(crate) fn drawn_reading(
    foil: &FoilingData,
    column: FoilColumn,
    row: u8,
) -> Option<crate::Reading> {
    let row = i32::from(row);
    let values = match column {
        // `draw_foiling` walks AXIS with the array, so row 1 is index 0.
        FoilColumn::Pitch => &foil.pitch[..],
        FoilColumn::Roll => &foil.roll[..],
        FoilColumn::Mid | FoilColumn::Right => {
            return stacked_cell(column, foil, row).and_then(|(v, _)| v.get().copied());
        }
        FoilColumn::Slot => return None,
    };
    values.get(usize::try_from(row - 1).ok()?)?.get().copied()
}

/// Which of a stacked column's value arrays a screen row belongs to, and its
/// index within that array.
///
/// Read off the block tables the renderer draws from, so the write side and the
/// draw side cannot disagree. `FoilingData::values_mut` carried its own copy of
/// these offsets until 2026-08-25, and it went stale the moment `REAR` lost a row
/// and moved down one: every rear parameter was stored one slot past the cell it
/// is drawn in, so the block showed its neighbour's number and `RKP` never showed
/// at all.
pub(crate) fn stacked_slot(column: FoilColumn, row: i32) -> Option<(usize, usize)> {
    blocks_of(column)?
        .iter()
        .enumerate()
        .find_map(|(block, table)| {
            let index = usize::try_from(row - table.first_row).ok()?;
            (index < table.rows.len()).then_some((block, index))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_rows() -> impl Iterator<Item = &'static Row> {
        AXIS.iter()
            .chain(HEIGHT.iter())
            .chain(REAR.iter())
            .chain(TURN.iter())
            .chain(MODE.iter())
            .chain(GLOBAL.iter())
    }

    /// A glyph a font lacks panics inside u8g2-fonts before `map_font_err` can
    /// see it, so every string this screen can draw is checked here rather than
    /// discovered on the water. The arrows are the ones at risk: they were added
    /// to `plex_small14_tf`'s subset for this screen, so this test fails until
    /// the blob is regenerated.
    #[test]
    fn every_glyph_this_screen_draws_exists() {
        for entry in all_rows() {
            let _ = width(&FONT_TINY, entry.hotkey);
            let _ = width(&FONT_TINY, entry.label);
        }
        // The config column's keys are not letters, and `]` is the newest of them.
        for (key, label) in SLOT_ACTIONS {
            let _ = width(&FONT_TINY, key);
            let _ = width(&FONT_TINY, label);
        }
        // Every heading, in the font that actually draws them: the bold blob is a
        // separate subset, so a glyph missing there panics just the same.
        for heading in ["Pitch", "Roll", "Configs"] {
            let _ = width(&FONT_HEADING, heading);
        }
        for block in MID_BLOCKS.iter().chain(RIGHT_BLOCKS.iter()) {
            // As drawn, not as stored: the title case is what puts lowercase
            // letters on screen.
            let _ = width(&FONT_HEADING, heading_title(block.name).as_str());
        }
        for extra in [
            "empty",
            "--",
            "--:--",
            "0123456789",
            "~",
            "PITCH RATE_P increased from 2.10 to 2.60",
            "HEIGHT CMD decreased from 5.0 to 4.5",
            "PITCH RATE_P increased from 7.98 to 8.00, clamped at max",
            "PITCH RATE_P already at max 8.00",
            "config 4 stored at 14:32",
            "config 4 restored from 14:32",
            "last change undone",
            "factory tune restored",
            "tune saved to flash",
            "5.0\u{2191} 8.0\u{2193}",
        ] {
            let _ = width(&FONT_TINY, extra);
        }
    }

    /// The datalogger's copy of the key and cursor map, which is generated from
    /// these tables by `support/export-foiling-params.py`. Checked here so the
    /// two cannot drift: the display renders whatever cell the datalogger
    /// selects, so a disagreement about which row a key means would put the
    /// cursor on one parameter while the operator adjusts another.
    ///
    /// Only the columns this file owns are compared. The ranges, steps and locks
    /// in the export are the boat's spec and are not the display's business.
    #[test]
    fn the_exported_map_matches_these_tables() {
        let csv = include_str!("../../../FOILING_PARAMETERS.csv");
        let mut exported: heapless::Vec<(&str, &str, u8, &str, usize), 80> = heapless::Vec::new();
        for line in csv.lines().skip(1) {
            let f: heapless::Vec<&str, 16> = line.split(',').collect();
            // Config slots carry no parameter row, so they have nothing to check.
            if f[5] == "CONFIG" {
                continue;
            }
            let cell = (
                f[0],
                f[1],
                f[3].parse().unwrap(),
                f[4],
                f[13].parse().unwrap(),
            );
            // The axis tables list one row per axis; both must agree with AXIS.
            if !exported.contains(&cell) {
                exported.push(cell).unwrap();
            }
        }

        let mut expected: heapless::Vec<(&str, &str, u8, &str, usize), 80> = heapless::Vec::new();
        for (index, entry) in AXIS.iter().enumerate() {
            for column in ["Pitch", "Roll"] {
                expected
                    .push((
                        entry.hotkey,
                        column,
                        1 + index as u8,
                        entry.label,
                        entry.decimals,
                    ))
                    .unwrap();
            }
        }
        for (blocks, column) in [
            (MID_BLOCKS.as_slice(), "Mid"),
            (RIGHT_BLOCKS.as_slice(), "Right"),
        ] {
            for block in blocks {
                for (index, entry) in block.rows.iter().enumerate() {
                    expected
                        .push((
                            entry.hotkey,
                            column,
                            (block.first_row + index as i32) as u8,
                            entry.label,
                            entry.decimals,
                        ))
                        .unwrap();
                }
            }
        }

        for cell in &expected {
            assert!(
                exported.contains(cell),
                "{cell:?} is drawn but missing from FOILING_PARAMETERS.csv -- \
                 rerun support/export-foiling-params.py"
            );
        }
        for cell in &exported {
            assert!(
                expected.contains(cell),
                "{cell:?} is in FOILING_PARAMETERS.csv but not drawn -- \
                 rerun support/export-foiling-params.py"
            );
        }
        assert_eq!(exported.len(), expected.len());
    }

    /// The exported index-to-cell mapping must agree with `cell_for_index`, or a
    /// parameter would land in one place on the display and be described as
    /// another in the datalogger's copy.
    #[test]
    fn the_exported_indices_match_cell_for_index() {
        let csv = include_str!("../../../FOILING_PARAMETERS.csv");
        let mut checked = 0;
        for line in csv.lines().skip(1) {
            let f: heapless::Vec<&str, 20> = line.split(',').collect();
            if f[7].is_empty() {
                continue; // config slots and the pitch-only gap carry no index
            }
            let column = match f[1] {
                "Pitch" => FoilColumn::Pitch,
                "Roll" => FoilColumn::Roll,
                "Mid" => FoilColumn::Mid,
                "Right" => FoilColumn::Right,
                other => panic!("unknown column {other:?}"),
            };
            let row: u8 = f[3].parse().unwrap();
            // A combined cell lists both halves, `22+23`.
            for (half, raw) in f[7].split('+').enumerate() {
                let index: u8 = raw.parse().unwrap();
                let (got_column, got_row, got_half) =
                    cell_for_index(index).unwrap_or_else(|| panic!("index {index} has no cell"));
                assert_eq!(
                    (got_column, got_row),
                    (column, row),
                    "index {index} maps to {got_column:?} row {got_row}, but the \
                     export says {column:?} row {row}"
                );
                let expected = if f[7].contains('+') {
                    if half == 0 {
                        PairHalf::Up
                    } else {
                        PairHalf::Down
                    }
                } else {
                    PairHalf::Whole
                };
                assert_eq!(got_half, expected, "index {index} half");
                checked += 1;
            }
        }
        assert_eq!(
            checked, 50,
            "foil_tune.lua PROTO_VERSION 9 has 50 parameters the screen draws"
        );
    }

    /// Hotkeys must be unique across the whole screen: one keypress selects one
    /// cell, so a repeat would be ambiguous, and cycling is not an option on a
    /// panel that takes a second to redraw.
    #[test]
    fn hotkeys_are_unique() {
        let keys: heapless::Vec<&str, 64> = all_rows().map(|entry| entry.hotkey).collect();
        for (index, key) in keys.iter().enumerate() {
            assert!(!keys[..index].contains(key), "hotkey {key:?} is used twice");
        }
        assert_eq!(keys.len(), 35, "every parameter needs a key");
    }

    /// The number row belongs to the config slots, and lowercase is only
    /// distinguishable from its capital at this size by an ascender or descender.
    #[test]
    fn hotkeys_are_unambiguous() {
        for entry in all_rows() {
            let key = entry.hotkey;
            assert!(
                !matches!(key, "l" | "i" | "o" | "O"),
                "hotkey {key:?} reads as I, 1 or 0"
            );
            assert!(
                !matches!(
                    key,
                    "a" | "c" | "e" | "m" | "n" | "s" | "u" | "v" | "w" | "x" | "z"
                ),
                "lowercase {key:?} has no ascender or descender, so it reads as a \
                 small capital"
            );
        }
    }

    /// `render_aligned` clips rather than erroring, so a value that outgrows its
    /// space produces no warning at all. Ordinary values must fit their column.
    #[test]
    fn widest_values_fit_their_columns() {
        for (name, text, budget) in [
            ("axis pitch", "0.020", AXIS_PITCH_W),
            ("axis roll", "0.020", AXIS_ROLL_W),
            ("height KP", "2000", MID_VALUE_W),
            ("turn value", "-20.0", RIGHT_VALUE_W),
            ("slot label", "empty", SLOT_LABEL_W),
            (
                "status line",
                "PITCH RATE_IMAX decreased from 20.0 to 18.5",
                STATUS_W,
            ),
            (
                "clamped status line",
                "PITCH RATE_IMAX decreased from 20.0 to 18.5, clamped at min",
                STATUS_W,
            ),
        ] {
            let w = width(&FONT_TINY, text);
            assert!(w <= budget, "{name}: {text:?} is {w}px, budget {budget}px");
        }
    }

    /// A diverged pair is wider than its column and overflows leftward, which is
    /// only sound while it stays clear of the label beside it. Every row that can
    /// diverge is checked against the widest pair it can reach.
    #[test]
    fn diverged_values_clear_their_labels() {
        for (label, worst, span) in [
            // Pitch rate max, both halves 0..180.
            ("RMAX", "180\u{2191} 180\u{2193}", AXIS_PITCH_SPAN),
            // Pitch envelope, +1..10 up and -10..-1 down.
            ("LIMIT", "10\u{2191} 10\u{2193}", AXIS_PITCH_SPAN),
            // Height command clamps, +0.5..5 up and -8..-0.5 down.
            ("CMD", "5.0\u{2191} 8.0\u{2193}", MID_VALUE_SPAN),
        ] {
            let value = width(&FONT_TINY, worst);
            let needed = width(&FONT_TINY, label) + FIELD_GAP + value;
            assert!(
                needed <= span,
                "{label}: {worst:?} is {value}px and the label {}px, needing \
                 {needed}px of the {span}px span",
                width(&FONT_TINY, label)
            );
        }
    }

    /// The headings are set in a heavier weight than the rows, and weight *does*
    /// change advances in this variable font, so they are measured in the font that
    /// draws them rather than assumed to match the labels.
    ///
    /// Each is checked against what stands beside it on its own row: the two axis
    /// headings against each other, a block heading against the width of its
    /// column, and `Configs` against the right-hand table's heading, which is the
    /// nearest thing to it on the top row.
    #[test]
    fn headings_fit_beside_each_other() {
        let (pitch, roll) = (width(&FONT_HEADING, "Pitch"), width(&FONT_HEADING, "Roll"));
        assert!(
            AXIS_ROLL_R - roll >= AXIS_PITCH_R,
            "Roll's heading starts at {} and Pitch's ends at {AXIS_PITCH_R}",
            AXIS_ROLL_R - roll
        );
        assert!(
            AXIS_PITCH_R - pitch >= AXIS_LABEL_X,
            "Pitch's heading reaches back into the hotkey strip"
        );

        for (blocks, label_x, value_r) in [
            (MID_BLOCKS.as_slice(), MID_LABEL_X, MID_VALUE_R),
            (RIGHT_BLOCKS.as_slice(), RIGHT_LABEL_X, RIGHT_VALUE_R),
        ] {
            for block in blocks.iter().filter(|block| block.heading) {
                let title = heading_title(block.name);
                let w = width(&FONT_HEADING, title.as_str());
                assert!(
                    label_x + w <= value_r,
                    "{title} is {w}px and runs past its column"
                );
            }
        }

        let configs = width(&FONT_HEADING, "Configs");
        let turn = width(&FONT_HEADING, heading_title(RIGHT_BLOCKS[0].name).as_str());
        assert!(
            SLOT_KEY_R - configs >= RIGHT_LABEL_X + turn,
            "Configs starts at {} and Turn ends at {}",
            SLOT_KEY_R - configs,
            RIGHT_LABEL_X + turn
        );
    }

    /// Every label has to fit the column it is drawn in.
    #[test]
    fn widest_labels_fit_their_columns() {
        for (rows, budget, name) in [
            (AXIS.as_slice(), AXIS_LABEL_W, "axis"),
            (HEIGHT.as_slice(), MID_LABEL_W, "height"),
            (REAR.as_slice(), MID_LABEL_W, "rear"),
            (TURN.as_slice(), RIGHT_LABEL_W, "turn"),
            (MODE.as_slice(), RIGHT_LABEL_W, "mode"),
            (GLOBAL.as_slice(), RIGHT_LABEL_W, "global"),
        ] {
            for entry in rows {
                let w = width(&FONT_TINY, entry.label);
                assert!(
                    w <= budget,
                    "{name} label {:?} is {w}px, budget {budget}px",
                    entry.label
                );
            }
        }
    }

    /// The blocks must not overlap each other or the status line.
    #[test]
    fn blocks_do_not_collide() {
        for blocks in [MID_BLOCKS.as_slice(), RIGHT_BLOCKS.as_slice()] {
            for pair in blocks.windows(2) {
                let end = pair[0].first_row + pair[0].rows.len() as i32;
                let next_start = pair[1].first_row - i32::from(pair[1].heading);
                assert!(
                    end <= next_start,
                    "{} ends at row {end}, {} starts at {next_start}",
                    pair[0].name,
                    pair[1].name
                );
            }
            let last = blocks.last().unwrap();
            assert!(last.first_row + (last.rows.len() as i32) < ROWS);
        }
    }

    /// The inverted cell has to land on the row whose value was drawn, and a
    /// stacked block below the first is pushed down by `SECTION_GAP`. Placing the
    /// cursor on the bare row grid inverts the row above instead, which reads as
    /// two half-covered values rather than as a selection.
    #[test]
    fn the_cursor_sits_on_the_row_it_selects() {
        for (blocks, column) in [
            (MID_BLOCKS.as_slice(), FoilColumn::Mid),
            (RIGHT_BLOCKS.as_slice(), FoilColumn::Right),
        ] {
            for block in blocks {
                for index in 0..block.rows.len() as i32 {
                    let row = block.first_row + index;
                    assert_eq!(
                        cell_y(column, row),
                        row_y(row) + block.y_offset(),
                        "{} row {row}",
                        block.name
                    );
                }
            }
        }
        // Not vacuous: the second block really is off the grid.
        assert_ne!(
            cell_y(FoilColumn::Mid, MID_BLOCKS[1].first_row),
            row_y(MID_BLOCKS[1].first_row),
            "the rear block is offset, so its cursor must be too"
        );
        // The single-table columns are drawn straight off the grid.
        assert_eq!(cell_y(FoilColumn::Pitch, 13), row_y(13));
        assert_eq!(cell_y(FoilColumn::Slot, 12), row_y(12));
    }

    /// `locate` has to agree with what `draw_*` puts on screen, or the status
    /// line would name a different parameter than the one that changed.
    #[test]
    fn locate_matches_the_drawn_rows() {
        assert_eq!(locate(FoilColumn::Pitch, 1).unwrap().1.label, "RATE_P");
        assert_eq!(locate(FoilColumn::Roll, 13).unwrap().1.label, "RLL>PTCH");
        assert!(
            locate(FoilColumn::Pitch, 0).is_none(),
            "row 0 is the heading"
        );
        assert!(locate(FoilColumn::Pitch, 14).is_none());

        let (group, entry) = locate(FoilColumn::Mid, 1).unwrap();
        assert_eq!((group, entry.label), ("HEIGHT", "KP"));
        assert!(
            locate(FoilColumn::Mid, 8).is_none(),
            "row 8 is Rear's heading"
        );
        let (group, entry) = locate(FoilColumn::Mid, 9).unwrap();
        assert_eq!((group, entry.label), ("REAR", "RKP"));
        let (group, entry) = locate(FoilColumn::Right, 12).unwrap();
        assert_eq!((group, entry.label), ("GLOBAL", "SPEED"));
        assert!(
            locate(FoilColumn::Right, 7).is_none(),
            "row 7 is Mode's heading"
        );
    }

    fn edit(from: f32, to: f32, clamped: Option<FoilLimit>) -> FoilEdit {
        FoilEdit {
            column: FoilColumn::Pitch,
            row: 1,
            from,
            to,
            clamped,
        }
    }

    /// The three wordings, in the words the helm reads. The clamped ones exist
    /// because the number that lands is the parameter's bound and not what was
    /// asked for -- without them a key that does nothing looks like a dead panel.
    #[test]
    fn the_status_line_reports_a_clamped_write() {
        for (name, edit, expected) in [
            (
                "plain",
                edit(4.05, 4.07, None),
                "PITCH RATE_P increased from 4.05 to 4.07",
            ),
            (
                "clamped at the top",
                edit(7.98, 8.00, Some(FoilLimit::Max)),
                "PITCH RATE_P increased from 7.98 to 8.00, clamped at max",
            ),
            (
                "clamped at the bottom",
                edit(0.04, 0.02, Some(FoilLimit::Min)),
                "PITCH RATE_P decreased from 0.04 to 0.02, clamped at min",
            ),
            // Nothing moved: the cell was already on its limit, so there is no
            // movement to report and the line has to say why instead.
            (
                "pressed against the top",
                edit(8.00, 8.00, Some(FoilLimit::Max)),
                "PITCH RATE_P already at max 8.00",
            ),
            (
                "pressed against an unnamed bound",
                edit(8.00, 8.00, Some(FoilLimit::Unknown)),
                "PITCH RATE_P already at its limit 8.00",
            ),
        ] {
            assert_eq!(
                status_line(FoilEvent::Edit(edit))
                    .expect("row 1 exists")
                    .as_str(),
                expected,
                "{name}"
            );
        }
    }

    /// What a configuration key did, in the words the helm reads. The slot labels
    /// on screen are lowercase, so these are too.
    #[test]
    fn the_status_line_reports_a_slot_action() {
        let slot = |action, slot, time| FoilEvent::Slot(FoilSlotEvent { action, slot, time });
        for (name, event, expected) in [
            (
                "stored",
                slot(FoilConfigAction::Stored, 4, Some((14, 32))),
                Some("config 4 stored at 14:32"),
            ),
            // Restoring names the same timestamp, but it says *which* tune went
            // back rather than when anything happened, hence "from".
            (
                "restored",
                slot(FoilConfigAction::Restored, 9, Some((9, 15))),
                Some("config 9 restored from 09:15"),
            ),
            // Nobody stores a tune without GNSS speed, but the slot column has a
            // shape for it, so the line needs one too.
            (
                "stored with no time",
                slot(FoilConfigAction::Stored, 1, None),
                Some("config 1 stored"),
            ),
            (
                "undone",
                slot(FoilConfigAction::Undone, 0, None),
                Some("last change undone"),
            ),
            (
                "factory reset",
                slot(FoilConfigAction::FactoryReset, 0, None),
                Some("factory tune restored"),
            ),
            (
                "saved to flash",
                slot(FoilConfigAction::SavedToFlash, 0, None),
                Some("tune saved to flash"),
            ),
            // A slot the column does not have would put a sentence on screen about
            // a config the helm cannot see.
            (
                "a slot off the end",
                slot(FoilConfigAction::Stored, 10, None),
                None,
            ),
            (
                "an action from a later protocol",
                slot(FoilConfigAction::Unknown(9), 4, None),
                None,
            ),
        ] {
            let line = status_line(event);
            assert_eq!(line.as_ref().map(|line| line.as_str()), expected, "{name}");
        }
    }

    /// The sentence is composed into a fixed buffer and then right-aligned on the
    /// screen edge, so the longest one any cell can produce has to fit both. A
    /// buffer overrun drops the line entirely rather than truncating it, which is
    /// silent -- hence every cell rather than a hand-picked worst case.
    ///
    /// The numbers are each parameter's own range, from the export, so widening a
    /// range upstream shows up here as a failing test rather than as a sentence
    /// running off the edge of the screen.
    #[test]
    fn the_longest_status_line_fits() {
        let csv = include_str!("../../../FOILING_PARAMETERS.csv");
        for line in csv.lines().skip(1) {
            let f: heapless::Vec<&str, 16> = line.split(',').collect();
            // Config slots hold no parameter, and neither does the roll side of the
            // pitch-only cross-feed row; nothing can edit either.
            if f[5] == "CONFIG" || f[8].is_empty() {
                continue;
            }
            let column = match f[1] {
                "Pitch" => FoilColumn::Pitch,
                "Roll" => FoilColumn::Roll,
                "Mid" => FoilColumn::Mid,
                "Right" => FoilColumn::Right,
                other => panic!("unknown column {other:?} in the export"),
            };
            let row: u8 = f[3].parse().unwrap();
            let (min, max): (f32, f32) = (f[8].parse().unwrap(), f[9].parse().unwrap());

            // A write that moved names its bound, so `Unknown` cannot appear there;
            // a write that moved nothing carries only one number, and either end of
            // the range can be the one it is stuck at.
            let cases = [
                (min, max, None),
                (min, max, Some(FoilLimit::Max)),
                (min, min, Some(FoilLimit::Unknown)),
                (max, max, Some(FoilLimit::Unknown)),
            ];
            for (from, to, clamped) in cases {
                let mut edit = edit(from, to, clamped);
                (edit.column, edit.row) = (column, row);
                let line = status_line(FoilEvent::Edit(edit))
                    .unwrap_or_else(|| panic!("{column:?} row {row} overran the buffer"));
                let w = width(&FONT_TINY, line.as_str());
                assert!(w <= STATUS_W, "{line:?} is {w}px, budget {STATUS_W}px");
            }
        }

        // The slot lines are short by comparison, but they share the buffer and the
        // corner, so they are held to the same budget.
        for action in [
            FoilConfigAction::Stored,
            FoilConfigAction::Restored,
            FoilConfigAction::Undone,
            FoilConfigAction::FactoryReset,
            FoilConfigAction::SavedToFlash,
        ] {
            for slot in 1..=SLOT_COUNT as u8 {
                let event = FoilEvent::Slot(FoilSlotEvent {
                    action,
                    slot,
                    time: Some((23, 59)),
                });
                let line = status_line(event).expect("a slot line");
                let w = width(&FONT_TINY, line.as_str());
                assert!(w <= STATUS_W, "{line:?} is {w}px, budget {STATUS_W}px");
            }
        }
    }

    #[test]
    fn readings_render_and_collapse() {
        let mut buf: String<16> = String::new();
        format_reading(&mut buf, Some(&Reading::One(2.1)), 2);
        assert_eq!(buf.as_str(), "2.10");
        // A diverged pair shows both, the down value as a magnitude.
        format_reading(&mut buf, Some(&Reading::UpDown(5.0, -8.0)), 1);
        assert_eq!(buf.as_str(), "5.0\u{2191} 8.0\u{2193}");
        format_reading(&mut buf, None, 2);
        assert_eq!(buf.as_str(), "--");
    }
}
