//! Synthetic foiling data, for judging the layout without a bus.
//!
//! Nothing sends foiling parameters over CAN yet, so `--layout foiling` against a
//! real interface draws dashes in every cell -- which says nothing about whether
//! a gain is legible at 14px or whether a diverged pair reads clearly. This fills
//! the screen with values in the ranges the foil controller actually uses.
//!
//! Deliberately here and not in `draw-display`: it is a visual aid, not part of
//! the display's contract. When the CAN messages exist this goes away.

use draw_display::{
    DisplayData, FoilColumn, FoilConfigAction, FoilCursor, FoilEdit, FoilEvent, FoilLimit,
    FoilSlotEvent, FoilingData, Reading,
};

/// Plausible mid-tune values, in screen-row order per table.
const PITCH: [f32; 13] = [
    2.60, 0.60, 0.020, 1.20, 20.0, 0.65, 60.0, 6.0, 20.0, 20.0, 10.0, 60.0, 0.0,
];
const ROLL: [f32; 13] = [
    0.35, 0.10, 0.012, 0.55, 12.0, 0.50, 75.0, 14.0, 20.0, 20.0, 10.0, 60.0, 0.0,
];
const HEIGHT: [f32; 7] = [1200.0, 80.0, 900.0, 250.0, 0.45, 0.0, 2.40];
// Four, not five: `RTKI` (0.05) left the screen with the rear trim's I gain on
// 2026-08-25. `zip` against a 4-slot array truncates rather than failing, so a
// stale fifth entry silently shifted RSCALE/RSCHED/FRNTFF up a row here.
const REAR: [f32; 4] = [0.45, 0.85, 600.0, 0.15];
/// Bank limit at 20 deg, which is `TRN_MAX`'s own maximum -- the demo presses
/// against it below, and a stop the value is not actually sitting on would make
/// that a lie.
const TURN: [f32; 6] = [1.0, 20.0, 70.0, 20.0, 4.0, 0.0];
const MODE: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
const SPEED: f32 = 9.0;

/// The row the demo walks the cursor along, so the inversion and the status line
/// can both be seen changing.
const EDITED_ROW: u8 = 1;

/// The turn table's bank limit, and the value it is parked at: its maximum. The
/// second half of the cycle presses against it.
const CLAMPED_ROW: u8 = 4;
const CLAMPED_MAX: f32 = TURN[CLAMPED_ROW as usize - 1];

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

    // Each status-line wording in turn, because an edit can end three ways and a
    // configuration key is a fourth, and each has to be legible at this size.
    match (tick / 8) % 30 {
        // Walk the pitch P gain upwards a step at a time, as holding `+` would.
        // The edit's `from` stays put while `to` climbs, which is the behaviour the
        // status line exists to show.
        // From one step, not zero: the first frame is what `EG_SIMULATOR_DUMP`
        // captures, and a screenshot with no status line is not representative.
        walking @ 0..12 => {
            let from = PITCH[EDITED_ROW as usize - 1];
            let to = from + 0.05 * (walking + 1) as f32;
            foil.pitch[EDITED_ROW as usize - 1].update(Reading::One(to));
            foil.cursor.update(FoilCursor {
                column: FoilColumn::Pitch,
                row: EDITED_ROW,
            });
            foil.last_event = Some(FoilEvent::Edit(FoilEdit {
                column: FoilColumn::Pitch,
                row: EDITED_ROW,
                from,
                to,
                clamped: None,
            }));
        }
        // Then `+` held against the bank limit, which is already at its maximum:
        // first the press that reached the stop, then the ones that achieve
        // nothing. The number stops moving here, so the line is the only thing
        // left that says what the keyboard is doing.
        held @ 12..24 => {
            foil.cursor.update(FoilCursor {
                column: FoilColumn::Right,
                row: CLAMPED_ROW,
            });
            foil.last_event = Some(FoilEvent::Edit(FoilEdit {
                column: FoilColumn::Right,
                row: CLAMPED_ROW,
                from: if held < 18 {
                    CLAMPED_MAX - 0.5
                } else {
                    CLAMPED_MAX
                },
                to: CLAMPED_MAX,
                clamped: Some(FoilLimit::Max),
            }));
        }
        // And a configuration key, which the datalogger reports rather than the
        // flight controller: the same corner, a different sentence.
        _ => {
            foil.cursor.update(FoilCursor {
                column: FoilColumn::Pitch,
                row: EDITED_ROW,
            });
            foil.last_event = Some(FoilEvent::Slot(FoilSlotEvent {
                action: FoilConfigAction::Restored,
                slot: 2,
                time: Some((9, 15)),
            }));
        }
    }

    // Every branch above is the helm doing something, so the demo is a keyboard
    // that never stops. Said out loud because these fields were written into
    // rather than ingested -- see `FoilingData::note_event`.
    foil.note_event();
}
