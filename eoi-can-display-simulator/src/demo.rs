//! Synthetic foiling data, for judging the layout without a bus.
//!
//! Nothing sends foiling parameters over CAN yet, so `--layout foiling` against a
//! real interface draws dashes in every cell -- which says nothing about whether
//! a gain is legible at 14px or whether a diverged pair reads clearly. This fills
//! the screen with values in the ranges the foil controller actually uses.
//!
//! Deliberately here and not in `draw-display`: it is a visual aid, not part of
//! the display's contract. When the CAN messages exist this goes away.

use draw_display::{DisplayData, FoilColumn, FoilCursor, FoilEdit, FoilingData, Reading};

/// Plausible mid-tune values, in screen-row order per table.
const PITCH: [f32; 13] = [
    2.60, 0.60, 0.020, 1.20, 20.0, 0.65, 60.0, 6.0, 20.0, 20.0, 10.0, 60.0, 0.0,
];
const ROLL: [f32; 13] = [
    0.35, 0.10, 0.012, 0.55, 12.0, 0.50, 75.0, 14.0, 20.0, 20.0, 10.0, 60.0, 0.0,
];
const HEIGHT: [f32; 7] = [1200.0, 80.0, 900.0, 250.0, 0.45, 0.0, 2.40];
const REAR: [f32; 4] = [0.45, 0.85, 600.0, 0.15];
const TURN: [f32; 6] = [1.0, 20.0, 70.0, 8.0, 4.0, 0.0];
const MODE: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
const SPEED: f32 = 9.0;

/// The row the demo walks the cursor along, so the inversion and the status line
/// can both be seen changing.
const EDITED_ROW: u8 = 1;

/// Fill `data` with a plausible tune. `tick` advances once per redraw and drives
/// the one value that moves, so the screen looks live rather than frozen.
pub fn populate(data: &mut DisplayData, tick: u32) {
    let foil: &mut FoilingData = &mut data.foiling;

    for (slot, value) in foil.pitch.iter_mut().zip(PITCH) {
        slot.update(Reading::One(value));
    }
    for (slot, value) in foil.roll.iter_mut().zip(ROLL) {
        slot.update(Reading::One(value));
    }
    // The cross-feed has no roll counterpart, so it stays absent and draws a dash.
    foil.roll[12] = Default::default();

    for (slot, value) in foil.height.iter_mut().zip(HEIGHT) {
        slot.update(Reading::One(value));
    }
    // The one pair that diverges in service: nose-up authority is far smaller than
    // nose-down, which is what makes the arrows worth having.
    foil.height[5].update(Reading::UpDown(5.0, -8.0));

    for (slot, value) in foil.rear.iter_mut().zip(REAR) {
        slot.update(Reading::One(value));
    }
    for (slot, value) in foil.turn.iter_mut().zip(TURN) {
        slot.update(Reading::One(value));
    }
    for (slot, value) in foil.mode.iter_mut().zip(MODE) {
        slot.update(Reading::One(value));
    }
    foil.global[0].update(Reading::One(SPEED));

    // Two stored configs and one written without a fix, so all three slot states
    // appear at once.
    foil.slots[0].update(Some((14, 32)));
    foil.slots[1].update(Some((9, 15)));
    foil.slots[2].update(None);

    // Walk the pitch P gain upwards a step at a time, as holding `+` would. The
    // edit's `from` stays put while `to` climbs, which is the behaviour the status
    // line exists to show.
    // From one step, not zero: the first frame is what `EG_SIMULATOR_DUMP`
    // captures, and a screenshot with no status line is not representative.
    let steps = (tick / 8) % 12 + 1;
    let from = PITCH[EDITED_ROW as usize - 1];
    let to = from + 0.05 * steps as f32;
    foil.pitch[EDITED_ROW as usize - 1].update(Reading::One(to));
    foil.cursor.update(FoilCursor {
        column: FoilColumn::Pitch,
        row: EDITED_ROW,
    });
    foil.last_edit = Some(FoilEdit {
        column: FoilColumn::Pitch,
        row: EDITED_ROW,
        from,
        to,
    });
}
