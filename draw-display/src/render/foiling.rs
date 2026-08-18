//! The foiling screen: trim and tuning parameters, four tables wide.
//!
//! Geometry only -- the `Cell` model, the fonts and every widget come from
//! [`super`], exactly as [`super::dashboard`] uses them.
//!
//! Four regions tile the full width: the axis rate loops on the left with a
//! column per axis, the height and rear loops next, then turn/mode/global, then
//! the config slots hard against the right edge. A status line runs along the
//! bottom row, in the space the first three tables leave free.
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
use crate::{DisplayData, DisplayValue, FoilColumn, FoilingData, Reading};

/// One row of a table. `decimals` belongs to the parameter, not the value, so a
/// gain renders to the same precision whatever it currently reads.
struct Row {
    hotkey: &'static str,
    label: &'static str,
    decimals: usize,
}

const fn row(hotkey: &'static str, label: &'static str, decimals: usize) -> Row {
    Row {
        hotkey,
        label,
        decimals,
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

/// The axis rate loops. `RMAX` and `LIMIT` each collapse an up/down pair.
///
/// Labels keep the `RATE_` prefix the real parameters carry. Dropping it frees
/// 14px, but leaves rows reading `P  P` and a status line saying "PITCH P
/// increased", so the width comes out of `FIELD_GAP` instead.
const AXIS: [Row; 13] = [
    row("P", "RATE_P", 2),
    row("I", "RATE_I", 2),
    row("D", "RATE_D", 3),
    row("F", "RATE_FF", 2),
    row("M", "RATE_IMAX", 1),
    row("C", "TCONST", 2),
    row("R", "RMAX", 0),
    row("L", "LIMIT", 0),
    row("T", "FLT_T", 0),
    row("E", "FLT_E", 0),
    row("G", "FLT_D", 0),
    row("S", "SMAX", 0),
    row("X", "RLL>PTCH", 2),
];

/// The ride-height loop. `CMD` collapses `HYD_CMDMAX`/`HYD_CMDMIN`.
///
/// Lowercase mirrors [`AXIS`] where the concept is the same: `p`/`d` against
/// `P`/`D`. `k` stands in for the I gain because lowercase `i` is unusable beside
/// `I` and `1`, and `h` for IMAX because lowercase `m` would read as a small `M`.
const HEIGHT: [Row; 7] = [
    row("p", "KP", 0),
    row("k", "KI", 0),
    row("d", "KD", 0),
    row("h", "IMAX", 0),
    row("t", "TARGET", 2),
    row("g", "CMD", 1),
    row("b", "ARM", 2),
];

/// The rear foil: artificial tailplane, decalage and speed schedule.
const REAR: [Row; 4] = [
    row("K", "RKP", 2),
    row("W", "RSCALE", 2),
    row("Y", "RSCHED", 0),
    row("V", "FRNTFF", 2),
];

/// Coordinated-turn banking. `ENABLE` stays keyed as the in-flight kill switch.
const TURN: [Row; 6] = [
    row("N", "ENABLE", 0),
    row("U", "ON", 0),
    row("A", "FULL", 0),
    row("Z", "MAX", 1),
    row("H", "RATE", 1),
    row("J", "REV", 0),
];

/// Operating mode and the live test demands: `SCR_USER1..4` under names that mean
/// something, since the real ones do not.
const MODE: [Row; 4] = [
    row("y", "MODE", 0),
    row("q", "TEST_P", 1),
    row("f", "TEST_R", 1),
    row("B", "JOG", 0),
];

/// Gain scaling, which rescales both axes at once and so belongs to neither.
/// Drawn without a heading: one row does not earn a line of its own, and the
/// row it would have taken is the status line's.
const GLOBAL: [Row; 1] = [row("Q", "SPEED", 1)];

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

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// 14 rows: a heading row, twelve of parameters, and the status line sharing the
/// last one with the axis table's final entry.
const ROWS: i32 = 14;
/// Ink is 18px tall once descenders are counted -- `p`, `g`, `q` and `y` all
/// appear in hotkeys -- so 19px is the floor, not the 14px cap height.
const ROW_H: i32 = 19;
/// Centres the block, splitting the 6px of `272 - 14*19` top and bottom.
const BLOCK_TOP: i32 = (DISPLAY_HEIGHT as i32 - ROWS * ROW_H) / 2;
/// Measured ink height of a string with descenders.
const INK_H: i32 = 18;
/// Measured top of that ink box relative to a `VerticalPosition::Center` anchor.
/// Values are all digits, so their ink is only the cap band inside it -- which is
/// why a row-height box centred on the row misses their tops.
const INK_TOP: i32 = -9;
/// Padding around a value's cap band when its cell is inverted.
const CURSOR_PAD: i32 = 2;
const CURSOR_H: i32 = SMALL_CAP_H + 2 * CURSOR_PAD;

const _: () = assert!(ROW_H > INK_H, "rows would touch");
// The inverted cell must cover a value's ink without reaching the rows either
// side. Digits leave the descender space empty, which is where the slack is.
const _: () = assert!(CURSOR_H <= ROW_H);
const _: () = assert!(CURSOR_PAD + CURSOR_PAD + SMALL_CAP_H <= ROW_H);
const _: () = assert!(BLOCK_TOP >= 0);

/// Centre y of a screen row, the heading being row 0.
const fn row_y(index: i32) -> i32 {
    BLOCK_TOP + index * ROW_H + ROW_H / 2
}

/// Column widths, each the sum of its fields, and the gutters chosen so the four
/// tile the full width exactly.
const HOTKEY_W: i32 = 18;
/// Space between a table's fields. Tight, because an arrow costs 17px: a diverged
/// pitch pair needs a 110px column, and this is where that width came from.
/// Values are right-aligned so their left gap is usually far wider than this;
/// what it really sets is hotkey-to-label.
const FIELD_GAP: i32 = 5;

const AXIS_LABEL_W: i32 = 108;
/// Wide enough for `180^ 180v`, the widest a diverged pitch pair can reach.
const AXIS_PITCH_W: i32 = 110;
/// Roll has no up/down pairs -- `RLL2SRV_RMAX` and `ROLL_LIMIT_DEG` are single
/// values -- so this column never holds an arrow pair.
const AXIS_ROLL_W: i32 = 53;
const W_AXIS: i32 = HOTKEY_W + AXIS_LABEL_W + AXIS_PITCH_W + AXIS_ROLL_W + 3 * FIELD_GAP;

const MID_LABEL_W: i32 = 76;
/// The widest value column on the screen: `CMD` is the one diverged pair
/// carrying a decimal, so it has to hold `5.0^ 8.0v` at 96px.
const MID_VALUE_W: i32 = 96;
const W_MID: i32 = HOTKEY_W + MID_LABEL_W + MID_VALUE_W + 2 * FIELD_GAP;

const RIGHT_LABEL_W: i32 = 76;
const RIGHT_VALUE_W: i32 = 53;
const W_RIGHT: i32 = HOTKEY_W + RIGHT_LABEL_W + RIGHT_VALUE_W + 2 * FIELD_GAP;

const SLOT_LABEL_W: i32 = 61;
const SLOT_KEY_W: i32 = 13;
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
const AXIS_ROLL_R: i32 = AXIS_PITCH_R + FIELD_GAP + AXIS_ROLL_W;

const MID_HOTKEY_X: i32 = COLS[2].x;
const MID_LABEL_X: i32 = MID_HOTKEY_X + HOTKEY_W + FIELD_GAP;
const MID_VALUE_R: i32 = MID_LABEL_X + MID_LABEL_W + FIELD_GAP + MID_VALUE_W;

const RIGHT_HOTKEY_X: i32 = COLS[4].x;
const RIGHT_LABEL_X: i32 = RIGHT_HOTKEY_X + HOTKEY_W + FIELD_GAP;
const RIGHT_VALUE_R: i32 = RIGHT_LABEL_X + RIGHT_LABEL_W + FIELD_GAP + RIGHT_VALUE_W;

/// The slots mirror the parameter tables: their key sits at the far edge and the
/// label grows inward, so the two hotkey strips are the two screen edges.
const SLOT_KEY_R: i32 = COLS[6].right();
const SLOT_LABEL_R: i32 = SLOT_KEY_R - SLOT_KEY_W - SLOT_GAP;

/// The status line shares the bottom row, starting where the axis table ends.
/// Only the axis table reaches row 13, so everything right of it is free.
const STATUS_ROW: i32 = ROWS - 1;
const STATUS_X: i32 = COLS[2].x;
/// Only the test uses this, but it is the budget the sentence is written to.
#[cfg(test)]
const STATUS_W: i32 = DISPLAY_WIDTH as i32 - STATUS_X;

// Every table has to fit inside the rows, headings included, and none may reach
// into the status line except the axis table it shares a row with.
// The axis table starts on row 1, so its last row is the status line's.
const _: () = assert!(AXIS.len() as i32 == STATUS_ROW);
const _: () = assert!(MID_BLOCKS[1].first_row + REAR.len() as i32 <= STATUS_ROW);
const _: () = assert!(RIGHT_BLOCKS[2].first_row + GLOBAL.len() as i32 <= STATUS_ROW);
const _: () = assert!(1 + SLOT_COUNT as i32 + 2 <= STATUS_ROW);

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
            &FONT_SMALL,
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
    values: &[DisplayValue<Reading>],
    hotkey_x: i32,
    label_x: i32,
    value_r: i32,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    if block.heading {
        // Over the label column, not the values: these tables have one value
        // column, so the name belongs to the group rather than the numbers.
        let mut title: String<16> = String::new();
        for (index, ch) in block.name.chars().enumerate() {
            let ch = if index == 0 {
                ch
            } else {
                ch.to_ascii_lowercase()
            };
            title.push(ch).ok();
        }
        draw_text(
            display,
            &FONT_SMALL,
            HorizontalAlignment::Left,
            label_x,
            row_y(block.first_row - 1),
            title.as_str(),
        )?;
    }
    for (index, entry) in block.rows.iter().enumerate() {
        let y = row_y(block.first_row + index as i32);
        key_and_label(display, hotkey_x, label_x, y, entry)?;
        value(display, value_r, y, entry, &values[index], buf)?;
    }
    Ok(())
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
        &FONT_SMALL,
        HorizontalAlignment::Left,
        hotkey_x,
        y,
        entry.hotkey,
    )?;
    draw_text(
        display,
        &FONT_SMALL,
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
            write!(
                buf,
                "{up:.decimals$}\u{2191} {magnitude:.decimals$}\u{2193}"
            )
            .ok();
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
    reading: &DisplayValue<Reading>,
    buf: &mut String<16>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    format_reading(buf, reading.get(), entry.decimals);
    draw_text(
        display,
        &FONT_SMALL,
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
        &FONT_SMALL,
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
            &FONT_SMALL,
            HorizontalAlignment::Right,
            SLOT_LABEL_R,
            y,
            buf.as_str(),
        )?;
        buf.clear();
        write!(buf, "{}", index + 1).ok();
        draw_text(
            display,
            &FONT_SMALL,
            HorizontalAlignment::Right,
            SLOT_KEY_R,
            y,
            buf.as_str(),
        )?;
    }

    for (offset, (key, label)) in [("~", "undo"), ("0", "factory")].iter().enumerate() {
        let y = row_y(1 + SLOT_COUNT as i32 + offset as i32);
        draw_text(
            display,
            &FONT_SMALL,
            HorizontalAlignment::Right,
            SLOT_LABEL_R,
            y,
            label,
        )?;
        draw_text(
            display,
            &FONT_SMALL,
            HorizontalAlignment::Right,
            SLOT_KEY_R,
            y,
            key,
        )?;
    }
    Ok(())
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

/// The status line: what the last edit did, in words.
///
/// Held until the next edit replaces it -- there is no timeout, because the
/// point of the line is to still be readable a while after the change. The
/// sentence is composed here from the row tables, so nothing on the bus carries
/// text.
fn draw_status<D, C>(display: &mut D, foil: &FoilingData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let Some(edit) = foil.last_edit else {
        return Ok(());
    };
    let Some((group, entry)) = locate(edit.column, edit.row as i32) else {
        return Ok(());
    };

    let mut line: String<64> = String::new();
    let verb = if edit.to >= edit.from {
        "increased"
    } else {
        "decreased"
    };
    write!(
        &mut line,
        "{group} {} {verb} from {:.*} to {:.*}",
        entry.label, entry.decimals, edit.from, entry.decimals, edit.to
    )
    .ok();

    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Left,
        STATUS_X,
        row_y(STATUS_ROW),
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

    let y = row_y(row);
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
    FONT_SMALL
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

/// The array and index a stacked column's row draws from, so the inverted copy
/// reads the same value the first pass drew.
fn stacked_cell<'a>(
    column: FoilColumn,
    foil: &'a FoilingData,
    row: i32,
) -> Option<(&'a DisplayValue<Reading>, &'static Row)> {
    let (blocks, arrays): (&'static [Block], [&'a [DisplayValue<Reading>]; 3]) =
        if matches!(column, FoilColumn::Mid) {
            (&MID_BLOCKS, [&foil.height[..], &foil.rear[..], &[]])
        } else {
            (
                &RIGHT_BLOCKS,
                [&foil.turn[..], &foil.mode[..], &foil.global[..]],
            )
        };
    for (block, values) in blocks.iter().zip(arrays) {
        let index = usize::try_from(row - block.first_row).ok()?;
        if let (Some(value), Some(entry)) = (values.get(index), block.rows.get(index)) {
            return Some((value, entry));
        }
    }
    None
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
            let _ = width(&FONT_SMALL, entry.hotkey);
            let _ = width(&FONT_SMALL, entry.label);
        }
        for extra in [
            "Pitch",
            "Roll",
            "Height",
            "Rear",
            "Turn",
            "Mode",
            "Configs",
            "empty",
            "undo",
            "factory",
            "--",
            "--:--",
            "0123456789",
            "~",
            "PITCH RATE_P increased from 2.10 to 2.60",
            "HEIGHT CMD decreased from 5.0 to 4.5",
            "5.0\u{2191} 8.0\u{2193}",
        ] {
            let _ = width(&FONT_SMALL, extra);
        }
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
                    "a" | "c" | "e" | "m" | "n" | "r" | "s" | "u" | "v" | "w" | "x" | "z"
                ),
                "lowercase {key:?} has no ascender or descender, so it reads as a \
                 small capital"
            );
        }
    }

    /// `render_aligned` clips rather than erroring, so a value that outgrows its
    /// column produces no warning at all. These are the widest each column can
    /// reach in service.
    #[test]
    fn widest_values_fit_their_columns() {
        for (name, text, budget) in [
            ("axis pitch pair", "180\u{2191} 180\u{2193}", AXIS_PITCH_W),
            ("axis roll", "0.020", AXIS_ROLL_W),
            ("height CMD pair", "5.0\u{2191} 8.0\u{2193}", MID_VALUE_W),
            ("height KP", "2000", MID_VALUE_W),
            ("turn value", "100.0", RIGHT_VALUE_W),
            ("slot label", "empty", SLOT_LABEL_W),
            (
                "status line",
                "PITCH RATE_IMAX decreased from 20.0 to 18.5",
                STATUS_W,
            ),
        ] {
            let w = width(&FONT_SMALL, text);
            assert!(w <= budget, "{name}: {text:?} is {w}px, budget {budget}px");
        }
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
                let w = width(&FONT_SMALL, entry.label);
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
            assert!(last.first_row + last.rows.len() as i32 <= STATUS_ROW);
        }
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
        let (group, entry) = locate(FoilColumn::Mid, 9).unwrap();
        assert_eq!((group, entry.label), ("REAR", "RKP"));
        let (group, entry) = locate(FoilColumn::Right, 12).unwrap();
        assert_eq!((group, entry.label), ("GLOBAL", "SPEED"));
        assert!(
            locate(FoilColumn::Mid, 8).is_none(),
            "row 8 is Rear's heading"
        );
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
