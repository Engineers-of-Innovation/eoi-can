#![cfg_attr(not(test), no_std)]

mod render;
mod time;

/// Re-exported because [`FoilSlotEvent`] carries one: a consumer building or
/// matching on a slot event needs to name the action without depending on the
/// decoder directly.
pub use eoi_can_decoder::{FoilConfigAction, GnssDateTime};
pub use render::dashboard::draw_display;
pub use render::foiling::draw_foiling;
pub use render::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

use core::net::Ipv4Addr;

use eoi_can_decoder::{
    BatteryState, ChargeState, DataLoggerData, DischargeState, EoiBattery, EoiCanData, FoilConfig,
    FoilParamStatus, FoilSlot, FoilTuneValue, GanMpptPacket, GnssData, HeightSensorData,
    MpptChannel, MpptInfo, TemperatureData, ThrottleData, ThrottleErrors, VescData,
};
use mppt_layout::{gan_side_and_position, position_of, MpptKind, Side, GAN_STRAP_COUNT, LAYOUT};
use render::foiling::{cell_for_index, stacked_slot, PairHalf};

const MPPT_PANEL_COUNT: usize = LAYOUT.len();
use time::{Duration, Instant};

const DISPLAY_VALUE_TIMEOUT: Duration = Duration::from_secs(5);

/// Usable energy in the pack, in watt-hours.
///
/// Nothing on the bus reports it -- the BMS sends a state of charge and currents,
/// not a capacity -- so the display carries the boat's own number. It is the one
/// figure in the endurance estimate that comes from outside the bus, which is why
/// it is named here rather than folded into the arithmetic.
const PACK_CAPACITY_WH: f32 = 1450.0;

/// Time constant of the low-pass on the power the estimate divides by.
///
/// The raw draw follows the throttle, and an endurance figure that swung with it
/// would be unreadable: every puff of throttle would halve it. A minute is long
/// enough that the number holds still through a gust and short enough that
/// settling into a cruise is reflected within a leg. Charging is steadier and
/// would tolerate less smoothing, but there is no reason for the two to disagree.
const ENDURANCE_FILTER_TAU_S: f32 = 60.0;

/// Below this, in either direction, too little is moving for the estimate to mean
/// anything: the division blows up towards infinity and the answer stops being a
/// time. Under it the display shows dashes rather than a number nobody should
/// read.
const MIN_ENDURANCE_W: f32 = 10.0;

/// The longest endurance the clock can render: the field is three two-digit
/// groups, so 99:59:59 is the ceiling. Anything longer is not a useful reading
/// anyway -- it means barely anything is moving in or out.
const MAX_ENDURANCE_S: u32 = 99 * 3600 + 59 * 60 + 59;

/// Time constant of the low-pass on the heading.
///
/// Far shorter than the endurance's: this is smoothing GNSS jitter, not a
/// throttle, and it has to follow a real turn within a few seconds or the number
/// is lying about where the boat is pointing.
const HEADING_FILTER_TAU_S: f32 = 5.0;

/// Below this the course over ground is noise. The receiver derives a direction
/// from a position that is barely moving, so it swings tens of degrees between
/// frames -- and some senders report a flat 0 rather than nothing at all. Dashes
/// are the honest reading; a smoothed value would just be smoothed noise.
const MIN_HEADING_SPEED_KMH: f32 = 1.0;

/// A first-order low-pass over an irregularly sampled signal.
///
/// The CAN frames it is fed from arrive at a nominal rate, but a display that
/// misses frames, boots late, or is fed from a replayed log sees gaps. So the
/// smoothing is expressed as a time constant and the coefficient is computed per
/// sample from the actual interval, rather than the usual fixed alpha that
/// quietly changes meaning with the frame rate. A long gap gives an alpha near 1,
/// which snaps to the new sample -- the right answer, because after a gap the
/// filter's memory is of a different situation.
#[derive(Debug)]
struct LowPass {
    value: Option<f32>,
    last_sample: Instant,
}

impl Default for LowPass {
    fn default() -> Self {
        Self {
            value: None,
            last_sample: Instant::now(),
        }
    }
}

impl LowPass {
    /// Fold `sample` in, and return the filtered value.
    fn sample(&mut self, sample: f32, tau_s: f32) -> f32 {
        let dt_s = self.last_sample.elapsed().as_micros() as f32 / 1_000_000.0;
        self.last_sample = Instant::now();

        let filtered = match self.value {
            // `dt / (tau + dt)` rather than `1 - exp(-dt/tau)`: it needs no libm on
            // the firmware target, and it is bounded in (0, 1) for every dt, so no
            // interval -- however long or however short -- can make it overshoot.
            Some(previous) => previous + (dt_s / (tau_s + dt_s)) * (sample - previous),
            // Nothing to average with yet: start from the reading rather than from
            // zero, so the first estimate is not an hour of nonsense settling.
            None => sample,
        };
        self.value = Some(filtered);
        filtered
    }
}

/// A low-pass over a bearing, which does not average like an ordinary number: the
/// mean of 359 and 1 is 180, pointing exactly backwards.
///
/// Vector averaging is the usual answer and wants `sin`/`cos`, which are std-only
/// on the firmware. So this unwraps instead -- each sample is shifted by whole
/// turns until it lands within half a turn of the running value, smoothed on that
/// continuous line, and folded back onto the dial afterwards.
#[derive(Debug, Default)]
struct HeadingFilter(LowPass);

impl HeadingFilter {
    /// Fold in a bearing and return the smoothed one. `degrees` must be finite and
    /// already on 0..=360 -- [`DisplayData::update_heading`] is what guarantees it,
    /// and the folding below walks in whole turns, so a wild value would walk for a
    /// very long time.
    fn sample(&mut self, degrees: f32, tau_s: f32) -> f32 {
        let continuous = match self.0.value {
            Some(previous) => previous + shortest_turn(degrees - previous),
            None => degrees,
        };
        let smoothed = normalise_degrees(self.0.sample(continuous, tau_s));
        // Put the running value back on the dial, so however long the boat circles
        // the unwrap above never has more than one turn to undo.
        self.0.value = Some(smoothed);
        smoothed
    }
}

/// The difference between two bearings, taken the short way round: -180..180.
fn shortest_turn(mut delta: f32) -> f32 {
    while delta > 180.0 {
        delta -= 360.0;
    }
    while delta < -180.0 {
        delta += 360.0;
    }
    delta
}

/// Fold a bearing onto 0..360. At most one turn out, because everything reaching
/// here is either a checked sample or a smoothed value half a turn from one.
fn normalise_degrees(mut degrees: f32) -> f32 {
    while degrees >= 360.0 {
        degrees -= 360.0;
    }
    while degrees < 0.0 {
        degrees += 360.0;
    }
    degrees
}

/// How long a configuration-slot event keeps the status line to itself.
///
/// Restoring a slot writes the whole parameter table, so fifty read-backs arrive
/// within milliseconds and each one is, to the display, a parameter that changed.
/// Without this the line would report one of them instead of the restore that
/// caused them. The cost is that a keypress within this window does not get the
/// line, which at roughly a second per panel refresh is barely a frame or two.
const SLOT_EVENT_HOLD: Duration = Duration::from_secs(3);

/// Which screen to draw.
///
/// Both boards run the same decode pipeline into the same [`DisplayData`] and
/// differ only in the layout they render it with, so the choice is one value
/// rather than two code paths: a firmware bin fixes it at compile time, and the
/// simulator and framebuffer take it from `--layout` so a screen can be worked
/// on without flashing anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum Layout {
    /// The helm dashboard: speed, power, temperatures, times.
    #[default]
    Dashboard,
    /// Foiling trim and tuning parameters.
    Foiling,
}

impl Layout {
    /// Every layout, for a front-end to list in its `--help`.
    pub const ALL: [Self; 2] = [Self::Dashboard, Self::Foiling];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Foiling => "foiling",
        }
    }

    pub fn draw<D, C>(self, display: &mut D, data: &DisplayData) -> Result<(), D::Error>
    where
        D: embedded_graphics::prelude::DrawTarget<Color = C>,
        C: embedded_graphics::prelude::PixelColor
            + From<embedded_graphics::pixelcolor::BinaryColor>,
    {
        match self {
            Self::Dashboard => draw_display(display, data),
            Self::Foiling => draw_foiling(display, data),
        }
    }
}

impl core::fmt::Display for Layout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// The name was not one of [`Layout::ALL`].
///
/// Implements [`core::error::Error`] so clap's `FromStr` value parser accepts
/// [`Layout`] directly, without the front-ends each wrapping it in a local enum.
#[derive(Debug)]
pub struct UnknownLayout;

impl core::error::Error for UnknownLayout {}

impl core::fmt::Display for UnknownLayout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "expected one of")?;
        for (index, layout) in Layout::ALL.iter().enumerate() {
            write!(f, "{} {layout}", if index == 0 { "" } else { "," })?;
        }
        Ok(())
    }
}

impl core::str::FromStr for Layout {
    type Err = UnknownLayout;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|layout| layout.name() == s)
            .ok_or(UnknownLayout)
    }
}

mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

#[derive(Debug)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct DisplayValue<T> {
    value: Option<T>,
    last_updated: Instant,
}

impl<T> DisplayValue<T> {
    pub fn update(&mut self, value: T) {
        self.value = Some(value);
        self.last_updated = Instant::now();
    }

    pub fn is_valid(&self) -> bool {
        self.value.is_some() && self.last_updated.elapsed() < DISPLAY_VALUE_TIMEOUT
    }

    pub fn get(&self) -> Option<&T> {
        if self.is_valid() {
            self.value.as_ref()
        } else {
            None
        }
    }
}

impl<T> Default for DisplayValue<T> {
    fn default() -> Self {
        Self {
            value: None,
            last_updated: Instant::now(), // We need to set something as initial value, will be updated when first value is set
        }
    }
}

/// A value held until it is replaced, with no staleness timeout.
///
/// The counterpart to [`DisplayValue`], and the distinction is telemetry versus
/// configuration. A speed or a cell voltage that stops arriving is *unknown* --
/// drawing the last one would be a lie, so `DisplayValue` expires it. A tuning
/// parameter that stops arriving is *unchanged*: the flight controller is not
/// reporting it, it is storing it, and the value it last acknowledged is still
/// the value in effect.
///
/// So the foiling screen's parameter cells latch. `foil_tune.lua` is
/// event-driven -- one whole-table dump ~5 s after the flight controller boots,
/// and after that a `0x261` only as the ack for a `0x260` set or the reply to a
/// `0x262` request. Expiring these cells meant the screen went to dashes ~5 s
/// after that one burst and stayed there, with nothing on the bus that would
/// ever refresh it. Two mechanisms keep a latched cell honest instead:
///
/// - every set is acknowledged, so an edit is seen as it happens;
/// - the tuner re-requests the selected cell once the cursor settles, so a cell
///   is refreshed from the flight controller whenever the helm navigates to it.
///   A screen that missed the boot dump therefore heals cell by cell, starting
///   with the ones being tuned.
///
/// `None` still renders as dashes, so "never heard" stays distinguishable from a
/// real reading -- it is only *ageing out* that is gone.
///
/// A separate type rather than a second accessor on `DisplayValue`: which
/// semantics a cell has is then checked by the compiler instead of resting on
/// every call site picking the right getter.
#[derive(Debug)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct Latched<T> {
    value: Option<T>,
}

impl<T> Latched<T> {
    pub fn update(&mut self, value: T) {
        self.value = Some(value);
    }

    /// The last value received, or `None` if none ever was.
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }
}

impl<T> Default for Latched<T> {
    fn default() -> Self {
        Self { value: None }
    }
}

/// A parameter value: one number, or an up/down pair that has diverged.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum Reading {
    /// One number, or an up/down pair that currently agrees.
    One(f32),
    /// An up/down pair that has diverged, drawn as `up^ down v`.
    UpDown(f32, f32),
}

/// Which value column a cell is in. The row is the screen row, so the
/// datalogger addresses a cell without either side agreeing a parameter numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum FoilColumn {
    Pitch,
    Roll,
    /// The height and rear loops.
    Mid,
    /// Turn, mode and the global scaling speed.
    Right,
    /// A config slot.
    Slot,
}

/// The cell the datalogger has selected, drawn inverted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct FoilCursor {
    pub column: FoilColumn,
    /// Screen row, the header being row 0.
    pub row: u8,
}

/// Which end of a parameter's range a write ran into.
///
/// The flight controller clamps every write to the parameter's own min/max before
/// setting it, and says so in the `0x261` status byte -- but the byte says only
/// *that* it clamped, never which bound. Where the value moved, its direction
/// gives that away; where the cell was already sitting on its limit, nothing on
/// the bus says which way the helm pressed, and naming the wrong end would be
/// worse than naming neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum FoilLimit {
    Min,
    Max,
    /// Clamped against a bound that cannot be told apart from the bus alone.
    Unknown,
}

/// The last parameter change, for the status line.
///
/// `from` is the value before the current edit *burst*, not before the last
/// keypress: holding `+` walks `to` upwards while `from` stays put, so the line
/// reads as one movement rather than a stream of single steps. The datalogger
/// owns that distinction -- it decides when a burst starts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct FoilEdit {
    pub column: FoilColumn,
    pub row: u8,
    pub from: f32,
    pub to: f32,
    /// Set where the flight controller clamped the write. `to` is then the value
    /// that stuck, so a clamped edit with `from == to` is a keypress that moved
    /// nothing -- which is exactly the case the status line has to explain.
    pub clamped: Option<FoilLimit>,
}

/// A configuration slot action, for the status line.
///
/// The keyboard is on the datalogger and the slots live in its RAM, so this whole
/// struct is read from the bus rather than worked out here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub struct FoilSlotEvent {
    pub action: FoilConfigAction,
    /// The slot as it is labelled on screen, 1-9. Not meaningful for the actions
    /// that touch no slot.
    pub slot: u8,
    /// When the tune involved was stored, where that is known.
    pub time: Option<(u8, u8)>,
}

/// Whatever the status line is currently reporting.
///
/// One field rather than one per kind: the line has room for a single sentence, so
/// what it shows is simply the last thing that happened.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "defmt",
    cfg_attr(not(feature = "tokio"), derive(defmt::Format))
)]
pub enum FoilEvent {
    /// A parameter moved, or a write was clamped.
    Edit(FoilEdit),
    /// A configuration slot was stored or restored.
    Slot(FoilSlotEvent),
}

/// Everything the foiling screen draws.
///
/// Deliberately not one flat array indexed by ArduPilot parameter number: the
/// screen is addressed by column and row, so a renumbering upstream cannot
/// silently move a value to the wrong row.
#[derive(Debug, Default)]
pub struct FoilingData {
    /// The axis rate loops, one entry per screen row.
    ///
    /// [`Latched`], not [`DisplayValue`]: these are configuration the flight
    /// controller reports on change, not telemetry it streams. See `Latched`.
    pub pitch: [Latched<Reading>; 13],
    pub roll: [Latched<Reading>; 13],
    pub height: [Latched<Reading>; 7],
    pub rear: [Latched<Reading>; 4],
    pub turn: [Latched<Reading>; 6],
    pub mode: [Latched<Reading>; 4],
    pub global: [Latched<Reading>; 1],
    /// Store time per slot: `None` never stored, `Some(None)` stored without a
    /// GNSS fix, `Some(Some((h, m)))` stored at that time.
    ///
    /// Stays a [`DisplayValue`]: the datalogger republishes all nine at ~1 Hz, so
    /// a column that stops arriving really is unknown, and a slot wiped on the
    /// tuner has to go back to `empty` here.
    pub slots: [DisplayValue<Option<(u8, u8)>>; 9],
    /// Also a [`DisplayValue`]: this is where the helm is *now*, so the inverted
    /// highlight should not outlive the tuner that put it there.
    pub cursor: DisplayValue<FoilCursor>,
    /// Held until the next event arrives rather than timing out: the point of the
    /// line is to still be readable a while after the change.
    pub last_event: Option<FoilEvent>,
    /// When the last slot event arrived, which is how long its line is protected
    /// from the read-backs it caused. See [`SLOT_EVENT_HOLD`].
    slot_event_at: Option<Instant>,
    /// Both halves of each collapsed up/down pair, kept raw.
    ///
    /// A pair arrives as two separate `0x261` frames, so writing straight into
    /// the cell would lose whichever half came first. Indexed by
    /// [`FoilingData::PAIRS`]: pitch RMAX, pitch LIMIT, height CMD.
    pairs: [(Option<f32>, Option<f32>); 3],
}

impl FoilingData {
    /// The cells that hold an up/down pair, in `pairs` order.
    const PAIRS: [(FoilColumn, u8); 3] = [
        (FoilColumn::Pitch, 7),
        (FoilColumn::Pitch, 8),
        (FoilColumn::Mid, 6),
    ];

    fn values_mut(&mut self, column: FoilColumn, row: u8) -> Option<&mut Latched<Reading>> {
        let index = usize::from(row);
        match column {
            FoilColumn::Pitch => self.pitch.get_mut(index - 1),
            FoilColumn::Roll => self.roll.get_mut(index - 1),
            // The stacked columns are several tables deep, so the row is resolved
            // against the very block tables the renderer draws from -- not a copy
            // of their offsets, which is what went stale when `REAR` lost a row.
            FoilColumn::Mid | FoilColumn::Right => {
                let (block, index) = stacked_slot(column, i32::from(row))?;
                let array: &mut [Latched<Reading>] = match (column, block) {
                    (FoilColumn::Mid, 0) => &mut self.height,
                    (FoilColumn::Mid, 1) => &mut self.rear,
                    (FoilColumn::Right, 0) => &mut self.turn,
                    (FoilColumn::Right, 1) => &mut self.mode,
                    (FoilColumn::Right, 2) => &mut self.global,
                    _ => return None,
                };
                array.get_mut(index)
            }
            FoilColumn::Slot => None,
        }
    }

    /// Apply one `0x261` read-back.
    fn ingest_param(&mut self, index: u8, status: FoilParamStatus, value: f32) {
        let Some((column, row, half)) = cell_for_index(index) else {
            return;
        };

        // A status other than ok/clamped/locked is sent with a value of zero, so
        // the float is dropped rather than drawn: an unavailable `HYD_*` must read
        // as dashes, not as a gain of 0.
        let value = status.has_value().then_some(value);

        let reading = match half {
            PairHalf::Whole => value.map(Reading::One),
            _ => {
                let slot = Self::PAIRS
                    .iter()
                    .position(|&cell| cell == (column, row))
                    .expect("every paired index maps to a PAIRS cell");
                let pair = &mut self.pairs[slot];
                if matches!(half, PairHalf::Up) {
                    pair.0 = value;
                } else {
                    pair.1 = value;
                }
                // Published only once both halves are known: half a pair cannot be
                // drawn honestly, and the tuner dumps the whole table at connect,
                // so the gap is brief.
                match *pair {
                    (Some(up), Some(down)) => Some(Reading::UpDown(up, down)),
                    _ => None,
                }
            }
        };

        let Some(reading) = reading else {
            return;
        };

        // The status line's `from` is the value before this burst, so a run of
        // steps on one cell reads as a single movement. A different cell starts a
        // new burst.
        let clamped = matches!(status, FoilParamStatus::Clamped);
        if let Some(previous) = self.values_mut(column, row).and_then(|v| v.get().copied()) {
            let (was, is) = (first_of(previous), first_of(reading));
            let moved = (was - is).abs() > f32::EPSILON;
            // A slot event owns the line for a moment: the read-backs a restore
            // causes are exactly the ones that would otherwise replace it.
            let held = self
                .slot_event_at
                .as_ref()
                .is_some_and(|at| at.elapsed() < SLOT_EVENT_HOLD);
            // A clamped write is an edit even when nothing moved: the helm pressed a
            // key, the number stayed where it was, and the line is the only thing
            // that can say why. An unchanged *unclamped* read is not an edit -- the
            // tuner re-requests the selected cell while the cursor sits on it, and
            // the boot dump re-sends the whole table, so the same value arrives
            // repeatedly and every one of those would otherwise replace the message.
            if (moved || clamped) && !held {
                let burst = self
                    .last_edit()
                    .filter(|edit| edit.column == column && edit.row == row);
                // Which bound the write ran into, in order of how directly it is
                // known: this frame's own movement; failing that the burst that led
                // here, since a walk that was going up and has stopped is against
                // its maximum; failing that the verdict of the press that first
                // reached the stop. A burst that began against one says nothing
                // about which it is.
                let limit = clamped.then(|| {
                    bound(is, was)
                        .or_else(|| burst.and_then(|edit| bound(edit.to, edit.from)))
                        .or_else(|| burst.and_then(|edit| edit.clamped))
                        .unwrap_or(FoilLimit::Unknown)
                });
                self.last_event = Some(FoilEvent::Edit(FoilEdit {
                    column,
                    row,
                    from: burst.map_or(was, |edit| edit.from),
                    to: is,
                    clamped: limit,
                }));
            }
        }

        if let Some(slot) = self.values_mut(column, row) {
            slot.update(reading);
        }

        // The tuner re-requests the selected cell once the cursor settles, so the
        // most recent read-back is where it is. Fragile by construction -- during
        // the boot dump this walks the whole table before settling -- but it is the
        // mechanism the protocol offers, and the re-request corrects it within a
        // second. An explicit cursor frame would be better.
        //
        // That re-request is also what keeps a latched cell honest: navigating to a
        // cell refreshes it from the flight controller, so a screen that missed the
        // boot dump heals cell by cell. See [`Latched`].
        self.cursor.update(FoilCursor { column, row });
    }

    /// The last parameter edit, where that is what the line is showing. A slot
    /// event ends a burst: the next keypress is a movement of its own.
    fn last_edit(&self) -> Option<FoilEdit> {
        match self.last_event {
            Some(FoilEvent::Edit(edit)) => Some(edit),
            _ => None,
        }
    }

    /// Apply one configuration-slot message from the datalogger.
    fn ingest_config(&mut self, message: FoilConfig) {
        match message {
            FoilConfig::Slot { slot, contents } => {
                // Slots are numbered as they are labelled, from 1, so that the
                // datalogger's key and this index cannot disagree.
                let Some(cell) = slot
                    .checked_sub(1)
                    .and_then(|index| self.slots.get_mut(usize::from(index)))
                else {
                    return;
                };
                match contents {
                    // Cleared rather than timed out, so wiping a slot shows up on
                    // the next redraw instead of five seconds later.
                    FoilSlot::Empty => *cell = DisplayValue::default(),
                    FoilSlot::StoredAt(hour, minute) => cell.update(Some((hour, minute))),
                    // A state from a later protocol still means "not empty", which
                    // is the half of the label that matters: a slot drawn as empty
                    // is one the helm would overwrite without thinking.
                    FoilSlot::Stored | FoilSlot::Unknown(_) => cell.update(None),
                }
            }
            FoilConfig::Event {
                action,
                slot,
                contents,
            } => {
                // An action this build does not know is left alone: the line it
                // would replace is at least true.
                if matches!(action, FoilConfigAction::Unknown(_)) {
                    return;
                }
                self.last_event = Some(FoilEvent::Slot(FoilSlotEvent {
                    action,
                    slot,
                    time: match contents {
                        FoilSlot::StoredAt(hour, minute) => Some((hour, minute)),
                        _ => None,
                    },
                }));
                self.slot_event_at = Some(Instant::now());
            }
        }
    }
}

/// The bound a movement was heading for: up towards the maximum, down towards the
/// minimum, and neither if it did not move.
fn bound(to: f32, from: f32) -> Option<FoilLimit> {
    if to > from {
        Some(FoilLimit::Max)
    } else if to < from {
        Some(FoilLimit::Min)
    } else {
        None
    }
}

/// The number a reading leads with, for comparing one against another.
fn first_of(reading: Reading) -> f32 {
    match reading {
        Reading::One(v) | Reading::UpDown(v, _) => v,
    }
}

/// How long the pack has, and which way it is going.
///
/// One value rather than two fields, because the two are never both true and the
/// screen has one place to draw them. Which arrives decides what the block is
/// labelled, so the number and the word cannot contradict each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endurance {
    /// Seconds until empty at the present draw.
    ToEmpty(u32),
    /// Seconds until full at the present charge -- what the figure means once the
    /// BMS has switched discharge off, which is how the boat sits on the charger.
    ToFull(u32),
}

#[derive(Debug, Default)]
pub struct DisplayData {
    pub speed_kmh: DisplayValue<f32>,
    /// Course over ground in degrees, from the same frame as the speed. Not a
    /// compass bearing: it is where the boat is *going*, which drifting sideways
    /// or sitting still is not where it points.
    pub heading_deg: DisplayValue<f32>,
    pub gnss_fix: DisplayValue<GnssFix>,
    pub battery_state_of_charge: DisplayValue<f32>,
    /// How long the pack has left, in seconds, and in which direction -- derived
    /// here rather than reported by anything. See
    /// [`DisplayData::update_endurance`].
    pub battery_endurance: DisplayValue<Endurance>,
    pub battery_cell_voltages: [DisplayValue<f32>; 14],
    pub battery_current_pack: DisplayValue<f32>,
    pub battery_current_in: DisplayValue<f32>,
    pub battery_current_out_motor: DisplayValue<f32>,
    pub battery_current_out_peripherals: DisplayValue<f32>,
    pub battery_voltage: DisplayValue<f32>,
    pub battery_temperatures: [DisplayValue<i8>; 4],
    pub battery_uptime_ms: DisplayValue<u32>,
    pub battery_error_flags: DisplayValue<u32>,
    pub battery_balancing_status: DisplayValue<u16>,
    pub battery_state: DisplayValue<BatteryState>,
    pub battery_charge_state: DisplayValue<ChargeState>,
    pub battery_discharge_state: DisplayValue<DischargeState>,
    pub motor_battery_voltage: DisplayValue<f32>,
    pub motor_battery_current: DisplayValue<f32>,
    pub motor_current: DisplayValue<f32>,
    pub motor_duty_cycle: DisplayValue<f32>,
    pub motor_rpm: DisplayValue<i32>,
    pub motor_fet_temperature: DisplayValue<f32>,
    /// The VESC's own motor NTC input, from status message 4. Recorded because the
    /// VESC still sends it, but **not shown**: the reading is broken on this boat,
    /// which is why the standalone node below exists.
    pub vesc_motor_temperature: DisplayValue<f32>,
    /// The standalone motor NTC node, `0x219` -- the motor temperature the display
    /// shows.
    ///
    /// Nested `Option` on purpose. The outer one is the node's liveness, the inner
    /// one is whether the reading it sent was usable, and the display needs both:
    /// silence and a reported fault are different facts even though they draw the
    /// same dashes.
    pub motor_ntc_temperature: DisplayValue<Option<f32>>,
    pub throttle_value: DisplayValue<f32>,
    pub throttle_errors: DisplayValue<ThrottleErrors>,
    pub mppt_panel_info: [DisplayValue<(f32, f32, f32)>; MPPT_PANEL_COUNT], // (Power, Voltage, Current), indexed by boat-position - 1
    /// Hottest temperature each MPPT reports, in °C, indexed by ID strap -- not by
    /// boat position, so a unit reports regardless of whether `LAYOUT` places it.
    /// GaN units report a board and a heat sink temperature; this keeps the higher
    /// of the two. Legacy MPPTs have no strap and are not covered.
    pub mppt_temperatures: [DisplayValue<i8>; GAN_STRAP_COUNT],
    pub charging_disabled: DisplayValue<bool>,
    pub time: DisplayValue<GnssDateTime>,
    pub ip_address: DisplayValue<Ipv4Addr>,
    pub display_state_of_charge: DisplayValue<f32>,
    pub display_is_charging: DisplayValue<bool>,
    pub height_sensor_front_left: DisplayValue<u16>,
    pub height_sensor_front_right: DisplayValue<u16>,
    pub temperature_height_sensors_controller: DisplayValue<f32>,
    pub temperature_rudder_controller: DisplayValue<f32>,
    /// Only the foiling layout draws these; the dashboard ignores them.
    pub foiling: FoilingData,
    /// Smoothing state behind `battery_endurance`, one per direction. Private:
    /// they are the estimate's working memory, not readings, and nothing should
    /// draw them. Kept apart so switching to the charger does not hand the charge
    /// figure a minute of remembered discharge.
    drain_filter: LowPass,
    charge_filter: LowPass,
    /// Smoothing state behind `heading_deg`, private for the same reason.
    heading_filter: HeadingFilter,
}

impl DisplayData {
    pub fn ingest_eoi_can_data(&mut self, data: EoiCanData) {
        match data {
            EoiCanData::FoilTune(value) => match value {
                FoilTuneValue::Param {
                    index,
                    status,
                    value,
                } => {
                    // The status byte is passed on whole: it is what says whether a
                    // frame carries a reading at all, and whether the write behind
                    // it was clamped.
                    self.foiling.ingest_param(index, status, value);
                }
                // Neither is a parameter, and the display needs neither: the
                // version is the tuner's concern and the dump marker only brackets
                // a burst.
                FoilTuneValue::ProtocolVersion(_) | FoilTuneValue::DumpComplete(_) => {}
            },
            EoiCanData::FoilConfig(message) => self.foiling.ingest_config(message),
            EoiCanData::EoiBattery(eoi_battery) => match eoi_battery {
                EoiBattery::ChargeAndDischargeCurrent(data) => {
                    self.battery_current_in.update(data.charge_current);
                    self.battery_current_out_motor
                        .update(data.discharge_current);
                    self.update_endurance();
                }
                EoiBattery::SocErrorFlagsAndBalancing(data) => {
                    self.battery_state_of_charge.update(data.state_of_charge);
                    self.battery_error_flags.update(data.error_flags);
                    self.battery_balancing_status.update(data.balancing_status);
                }
                EoiBattery::PackAndPerriCurrent(data) => {
                    self.battery_current_out_peripherals
                        .update(data.perri_current);
                    self.battery_current_pack.update(data.pack_current);
                }
                EoiBattery::CellVoltages1_4(data) => {
                    self.update_cell_voltages(0, data.cell_voltage.as_slice());
                }
                EoiBattery::CellVoltages5_8(data) => {
                    self.update_cell_voltages(4, data.cell_voltage.as_slice());
                }
                EoiBattery::CellVoltages9_12(data) => {
                    self.update_cell_voltages(8, data.cell_voltage.as_slice());
                }
                EoiBattery::CellVoltages13_14PackAndStack(data) => {
                    self.update_cell_voltages(12, data.cell_voltage.as_slice());
                    self.battery_voltage.update(data.pack_voltage);
                }
                EoiBattery::TemperaturesAndStates(data) => {
                    for (index, value) in data.temperatures.iter().enumerate() {
                        self.battery_temperatures[index].update(*value);
                    }
                    self.battery_state.update(data.battery_state);
                    self.battery_charge_state.update(data.charge_state);
                    self.battery_discharge_state.update(data.discharge_state);
                }
                EoiBattery::BatteryUptime(data) => {
                    self.battery_uptime_ms.update(data.uptime_ms);
                }
            },

            EoiCanData::Throttle(throttle) => {
                if let ThrottleData::Status(data) = throttle {
                    self.throttle_value.update(data.value);
                    self.throttle_errors.update(data.error);
                }
            }

            EoiCanData::Vesc(vesc) => match vesc {
                VescData::StatusMessage1 {
                    rpm,
                    total_current,
                    duty_cycle,
                } => {
                    self.motor_rpm.update(rpm);
                    self.motor_current.update(total_current);
                    self.motor_duty_cycle.update(duty_cycle);
                }
                VescData::StatusMessage4 {
                    fet_temp,
                    motor_temp,
                    total_input_current,
                    current_pid_position: _,
                } => {
                    self.motor_battery_current.update(total_input_current);
                    self.motor_fet_temperature.update(fet_temp);
                    self.vesc_motor_temperature.update(motor_temp);
                }
                VescData::StatusMessage5 {
                    input_voltage,
                    tachometer: _,
                } => {
                    self.motor_battery_voltage.update(input_voltage);
                }
                _ => {}
            },
            EoiCanData::Mppt(mppt_data) => {
                let node = mppt_data.node_id();
                let channel_power = match mppt_data.inner() {
                    MpptInfo::Channel0(MpptChannel::Power(p)) => Some((0u8, p)),
                    MpptInfo::Channel1(MpptChannel::Power(p)) => Some((1u8, p)),
                    MpptInfo::Channel2(MpptChannel::Power(p)) => Some((2u8, p)),
                    MpptInfo::Channel3(MpptChannel::Power(p)) => Some((3u8, p)),
                    // Legacy MPPTs report a node temperature, but they have no ID
                    // strap to name them by, so it is not surfaced.
                    _ => None,
                };
                if let Some((channel, power)) = channel_power {
                    if let Some(pos) = position_of(MpptKind::Legacy { node, channel }) {
                        self.mppt_panel_info[pos as usize - 1].update((
                            power.voltage_in * power.current_in,
                            power.voltage_in,
                            power.current_in,
                        ));
                    }
                }
            }
            EoiCanData::Gnss(gnss) => match gnss {
                GnssData::GnssSpeedAndHeading(speed_kmh, heading_deg) => {
                    self.speed_kmh.update(speed_kmh);
                    self.update_heading(speed_kmh, heading_deg);
                }
                GnssData::GnssDateTime(data) => self.time.update(data),
                GnssData::GnssStatus(data) => {
                    self.gnss_fix.update(GnssFix::from_code(data.fix));
                }
                GnssData::GnssLatitude(_) => {}
                GnssData::GnssLongitude(_) => {}
            },
            EoiCanData::RudderController(_) => {}
            EoiCanData::HeightSensors(height) => match height {
                HeightSensorData::FrontLeft(status) => {
                    self.height_sensor_front_left.update(status.value);
                }
                HeightSensorData::FrontRight(status) => {
                    self.height_sensor_front_right.update(status.value);
                }
                _ => {}
            },
            EoiCanData::GanMppt(gan_data) => {
                let node = gan_data.node_id();
                match gan_data.inner() {
                    GanMpptPacket::Power(power) => {
                        if let Some(pos) = position_of(MpptKind::Gan { node }) {
                            self.mppt_panel_info[pos as usize - 1].update((
                                power.input_voltage * power.input_current,
                                power.input_voltage,
                                power.input_current,
                            ));
                        }
                    }
                    GanMpptPacket::Status(status) => {
                        // Indexed by strap: every MPPT on the bus reports, whether
                        // or not LAYOUT gives it a boat position. The heat sink
                        // usually leads the board, so take whichever is hotter.
                        if let Some(slot) = self.mppt_temperatures.get_mut(node as usize) {
                            slot.update(status.board_temp.max(status.heat_sink_temp));
                        }
                    }
                    _ => {}
                }
            }
            EoiCanData::Temperature(temp) => match temp {
                TemperatureData::HeightSensorsController(value) => {
                    self.temperature_height_sensors_controller.update(value);
                }
                TemperatureData::RudderController(value) => {
                    self.temperature_rudder_controller.update(value);
                }
                TemperatureData::MotorNtc(ntc) => {
                    // Store the frame's verdict, fault included, so hearing from the
                    // node counts as fresh data even when it has no reading to give.
                    // The status flags say why, which the display has no room for.
                    self.motor_ntc_temperature.update(ntc.temperature);
                }
            },
            EoiCanData::DataLogger(DataLoggerData::WifiIp(octets)) => {
                self.ip_address.update(Ipv4Addr::from(octets));
            }
        }
    }

    pub fn update_cell_voltages(&mut self, offset: usize, values: &[f32]) {
        for (index, value) in values.iter().enumerate() {
            self.battery_cell_voltages[offset + index].update(*value);
        }
    }

    /// Fold a course-over-ground reading into the smoothed heading.
    ///
    /// Three things have to hold before a bearing reaches the screen, and a sample
    /// failing any of them is dropped rather than drawn: the reading has to be a
    /// number, it has to be a bearing, and the boat has to be moving fast enough
    /// for the receiver to have derived it from real movement.
    ///
    /// Dropping leaves `heading_deg` to age out into dashes, which is the whole
    /// point -- a receiver with no course to report is not pointing north.
    fn update_heading(&mut self, speed_kmh: f32, heading_deg: f32) {
        // A sender with nothing to say may send NaN. Formatting that would draw a
        // confident `000` -- the cast lands on zero -- and it would send the fold
        // below round forever.
        if !heading_deg.is_finite() || !speed_kmh.is_finite() {
            return;
        }
        // A course over ground is 0..360 by definition, so anything else is a
        // sender bug. Dropping it is safer than rescuing it: the fold walks in
        // whole turns, and a wild value would walk a very long way.
        if !(0.0..=360.0).contains(&heading_deg) {
            return;
        }
        // An exact zero is not a bearing on this bus, it is the sender saying it
        // has no course. Captured 2026-08-28: the autopilot interleaves
        // `00 00 00 00` with a course walking smoothly through the 320s, two
        // frames in ten, at 13 km/h -- so it is neither a slow-speed artefact nor
        // a reading. A course computed as a float is never exactly zero anyway:
        // due north arrives as 359.87 or 0.14, and dropping the one frame in
        // billions that lands on the nose costs nothing next to drawing north
        // while the boat sails south.
        if heading_deg == 0.0 {
            return;
        }
        if speed_kmh < MIN_HEADING_SPEED_KMH {
            return;
        }

        let smoothed = self
            .heading_filter
            .sample(normalise_degrees(heading_deg), HEADING_FILTER_TAU_S);
        self.heading_deg.update(smoothed);
    }

    /// Whether the BMS has switched the discharge path off.
    ///
    /// That is the boat on the charger: the pack takes current and gives none, so
    /// the endurance question turns round -- "how long until I can go out" rather
    /// than "how long can I stay out". Anything the BMS reports other than a
    /// conducting discharge path counts, including its fault states, because none
    /// of them will be emptying the pack either.
    ///
    /// Silence does not count. A display that has not heard from the BMS assumes
    /// the boat is sailing, which is what it is doing whenever anybody is reading
    /// this screen.
    pub fn discharge_is_off(&self) -> bool {
        self.battery_discharge_state
            .get()
            .is_some_and(|state| !matches!(state, DischargeState::On | DischargeState::PreChargeOn))
    }

    /// Re-estimate how long the pack has, in whichever direction it is going.
    ///
    /// Driven from `0x101` rather than a timer: it is the frame carrying the motor
    /// current, it arrives at a steady rate, and running here keeps the estimate in
    /// the ingest path where every other derived value already lives -- the render
    /// stays a pure function of what has been received.
    ///
    /// The direction comes from the BMS's own discharge state rather than from
    /// which way the current happens to be flowing. On a sunny reach the panels
    /// can out-produce the motor for a few seconds at a time, and a figure that
    /// flipped between "to empty" and "to full" with the clouds would be useless.
    ///
    /// A sample is skipped rather than faked when a reading is missing or stale,
    /// and nothing is published when too little is moving to divide by. Both leave
    /// `battery_endurance` to age out into dashes, which is the honest reading:
    /// not "forever", but "not known".
    fn update_endurance(&mut self) {
        let (Some(&voltage), Some(&state_of_charge)) = (
            self.battery_voltage.get(),
            self.battery_state_of_charge.get(),
        ) else {
            return;
        };
        let charged = (state_of_charge / 100.0).clamp(0.0, 1.0);

        // Emptying divides what is in the pack by what is leaving it; filling
        // divides what is missing by what is arriving. A full pack on the charger
        // reads 00:00:00, which is the right answer.
        let (power_w, energy_wh, endurance): (f32, f32, fn(u32) -> Endurance) =
            if self.discharge_is_off() {
                let Some(&current_in) = self.battery_current_in.get() else {
                    return;
                };
                // Charge current arrives positive, so this needs no negating --
                // only the floor, for a charger briefly drawing rather than giving.
                let charge_w = (voltage * current_in).max(0.0);
                (
                    self.charge_filter.sample(charge_w, ENDURANCE_FILTER_TAU_S),
                    (1.0 - charged) * PACK_CAPACITY_WH,
                    Endurance::ToFull,
                )
            } else {
                let (Some(&motor), Some(&peripherals)) = (
                    self.battery_current_out_motor.get(),
                    self.battery_current_out_peripherals.get(),
                ) else {
                    return;
                };
                // Currents leaving the battery are negative on the bus, so the draw
                // is the negated sum. Clamped at zero because a regenerating motor
                // is not a negative drain -- it is a charge, and this direction does
                // not count those.
                //
                // The draw is **power out**, not net power: the solar input is left
                // out on purpose, so the figure answers "how long if the sun stops"
                // and errs short. Counting the panels in would need only
                // `battery_current_in` added to the sum, at the cost of an endurance
                // that grows when a cloud passes.
                let drain_w = (-(voltage * (motor + peripherals))).max(0.0);
                (
                    self.drain_filter.sample(drain_w, ENDURANCE_FILTER_TAU_S),
                    charged * PACK_CAPACITY_WH,
                    Endurance::ToEmpty,
                )
            };

        if power_w < MIN_ENDURANCE_W {
            return;
        }
        let seconds = energy_wh / power_w * 3600.0;
        self.battery_endurance
            .update(endurance((seconds as u32).min(MAX_ENDURANCE_S)));
    }

    /// The hottest MPPT currently reporting, and how to name it. `None` while no
    /// MPPT has sent a temperature.
    pub fn hottest_mppt(&self) -> Option<(MpptId, i8)> {
        self.mppt_temperatures
            .iter()
            .enumerate()
            .filter_map(|(strap, value)| value.get().map(|t| (MpptId::of_strap(strap as u8), *t)))
            .max_by_key(|(_, t)| *t)
    }

    /// Motor temperature in °C, from the standalone `0x219` node.
    ///
    /// The VESC's own reading is deliberately not a fallback. It is broken, so
    /// falling back to it would replace honest dashes with a wrong number -- and a
    /// wrong motor temperature is worse than none, since this is what the
    /// over-temperature icon is decided on.
    pub fn motor_temperature(&self) -> Option<f32> {
        self.motor_ntc_temperature.get().copied().flatten()
    }

    /// The hottest of the battery's four pack thermistors, in °C.
    pub fn hottest_battery_temperature(&self) -> Option<i8> {
        self.battery_temperatures
            .iter()
            .filter_map(|value| value.get().copied())
            .max()
    }
}

/// GNSS fix quality, from byte 0 of `0x200`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GnssFix {
    None,
    /// Latitude and longitude only.
    Fix2D,
    Fix3D,
}

impl GnssFix {
    /// Codes are as `CAN_MESSAGES.md` documents them: 1 is 3D and 2 is 2D, which
    /// looks backwards but keeps 1 meaning what it always did. Anything else is
    /// treated as no fix rather than guessed at.
    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Fix3D,
            2 => Self::Fix2D,
            _ => Self::None,
        }
    }
}

/// How an MPPT is named on screen: the side and 0-based position its ID strap
/// encodes -- `F0`-`F7` forward, `R0`-`R7` aft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpptId {
    pub side: Side,
    pub position: u8,
}

impl MpptId {
    fn of_strap(strap: u8) -> Self {
        let (side, position) = gan_side_and_position(strap);
        Self { side, position }
    }
}

#[cfg(test)]
mod foil_ingest_tests {
    use super::*;
    use embedded_can::{Id, StandardId};
    use eoi_can_decoder::can_frame::CanFrame;
    use eoi_can_decoder::parse_eoi_can_data;

    /// One `0x261 PARAM_VALUE` frame, as `foil_tune.lua` sends it.
    fn value_frame(index: u8, status: u8, value: f32) -> CanFrame {
        let bytes = value.to_le_bytes();
        CanFrame::from_encoded(
            Id::Standard(StandardId::new(0x261).unwrap()),
            &[index, status, bytes[0], bytes[1], bytes[2], bytes[3]],
        )
    }

    fn feed(data: &mut DisplayData, index: u8, status: u8, value: f32) {
        let parsed = parse_eoi_can_data(&value_frame(index, status, value)).expect("decodes");
        data.ingest_eoi_can_data(parsed);
    }

    /// Any frame by ID and bytes, for the slot messages the datalogger sends.
    fn feed_raw(data: &mut DisplayData, id: u16, bytes: &[u8]) {
        let frame = CanFrame::from_encoded(Id::Standard(StandardId::new(id).unwrap()), bytes);
        data.ingest_eoi_can_data(parse_eoi_can_data(&frame).expect("decodes"));
    }

    /// One `0x201`, as the autopilot sends it: speed then course, both LE f32.
    fn feed_heading(data: &mut DisplayData, speed_kmh: f32, heading_deg: f32) {
        let speed = speed_kmh.to_le_bytes();
        let heading = heading_deg.to_le_bytes();
        feed_raw(
            data,
            0x201,
            &[
                speed[0], speed[1], speed[2], speed[3], heading[0], heading[1], heading[2],
                heading[3],
            ],
        );
    }

    /// The wrap the filter exists for. A boat holding north sends bearings either
    /// side of 360, and an ordinary average of 359 and 1 is 180 -- the reading
    /// would swing to due south while the boat sailed straight.
    #[tokio::test(start_paused = true)]
    async fn a_course_either_side_of_north_never_swings_south() {
        let mut data = DisplayData::default();
        for degrees in [359.0, 1.0, 358.0, 2.0, 0.0, 359.5] {
            feed_heading(&mut data, 12.0, degrees);
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
            let smoothed = data.heading_deg.get().copied().expect("a heading");
            assert!(
                !(10.0..350.0).contains(&smoothed),
                "{degrees} deg smoothed to {smoothed}, which is the long way round"
            );
        }
    }

    /// Smoothing must not become lag: a real turn has to arrive within a few
    /// seconds, or the number is lying about where the boat points.
    #[tokio::test(start_paused = true)]
    async fn a_turn_arrives_within_a_few_seconds() {
        let mut data = DisplayData::default();
        feed_heading(&mut data, 12.0, 90.0);
        assert_eq!(data.heading_deg.get().copied(), Some(90.0), "first sample");

        // Hard turn to 180, held. One time constant should carry most of it.
        for _ in 0..5 {
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
            feed_heading(&mut data, 12.0, 180.0);
        }
        let after_tau = data.heading_deg.get().copied().expect("a heading");
        assert!(
            after_tau > 135.0,
            "five seconds -- one time constant -- into a 90 deg turn the reading \
             has only reached {after_tau}"
        );

        // Three time constants and it is there for reading purposes: the panel
        // draws whole degrees, and this is inside seven of them.
        for _ in 0..10 {
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
            feed_heading(&mut data, 12.0, 180.0);
        }
        let settled = data.heading_deg.get().copied().expect("a heading");
        assert!(
            settled > 173.0,
            "fifteen seconds after the turn the reading is still {settled}"
        );

        // And it never overshoots past the course being steered.
        assert!(settled <= 180.0, "overshot to {settled}");
    }

    /// Jitter is what gets filtered: a course wobbling either side of east must
    /// read as east, not follow every frame.
    #[tokio::test(start_paused = true)]
    async fn jitter_is_smoothed_away() {
        let mut data = DisplayData::default();
        for degrees in [90.0, 82.0, 98.0, 84.0, 96.0, 88.0] {
            feed_heading(&mut data, 12.0, degrees);
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
        }
        let smoothed = data.heading_deg.get().copied().expect("a heading");
        assert!(
            (86.0..94.0).contains(&smoothed),
            "a wobble around 90 smoothed to {smoothed}"
        );
    }

    /// Tied up or drifting, the course is derived from a position that is barely
    /// moving. Some senders report a flat 0 for it, which is what put a confident
    /// `000` on the panel -- so below the gate nothing is published at all.
    #[tokio::test(start_paused = true)]
    async fn a_course_below_walking_pace_is_not_published() {
        let mut data = DisplayData::default();
        feed_heading(&mut data, 0.2, 0.0);
        assert_eq!(data.heading_deg.get(), None, "0.2 km/h has no course");

        // Under way: published.
        feed_heading(&mut data, 12.0, 137.0);
        assert_eq!(data.heading_deg.get().copied(), Some(137.0));

        // Back alongside: the last good bearing ages out rather than being held or
        // replaced by the sender's zero.
        for _ in 0..6 {
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
            feed_heading(&mut data, 0.1, 0.0);
        }
        assert_eq!(
            data.heading_deg.get(),
            None,
            "a stale course must not stand"
        );
    }

    /// The capture from the boat, replayed frame for frame: a steady course
    /// through the 320s at 13 km/h with two exact zeros interleaved. Those are the
    /// sender saying it has no course, and taking them for bearings drags the
    /// reading round towards north while the boat holds its course.
    #[tokio::test(start_paused = true)]
    async fn the_autopilots_zero_frames_do_not_drag_the_course_north() {
        let mut data = DisplayData::default();
        for (speed_kmh, heading_deg) in [
            (13.30, 326.61),
            (13.20, 0.0),
            (13.40, 325.39),
            (13.40, 324.09),
            (13.60, 322.89),
            (13.70, 322.49),
            (13.70, 0.0),
            (13.30, 320.60),
            (13.00, 320.66),
            (12.60, 320.77),
        ] {
            feed_heading(&mut data, speed_kmh, heading_deg);
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
        }

        let smoothed = data.heading_deg.get().copied().expect("a heading");
        assert!(
            (318.0..330.0).contains(&smoothed),
            "the zero frames pulled the course to {smoothed}, off the 320s the \
             boat was actually steering"
        );
    }

    /// A sender with no course may say so with NaN. Formatted, that draws a
    /// confident `000` -- the cast lands on zero -- so it must never reach the
    /// screen. The infinities would also send the unwrap round forever.
    #[tokio::test(start_paused = true)]
    async fn a_course_that_is_not_a_bearing_is_dropped() {
        let mut data = DisplayData::default();
        feed_heading(&mut data, 12.0, 137.0);

        for nonsense in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -1.0,
            400.0,
            1.0e9,
        ] {
            feed_heading(&mut data, 12.0, nonsense);
            assert_eq!(
                data.heading_deg.get().copied(),
                Some(137.0),
                "{nonsense} reached the heading instead of being dropped"
            );
        }
    }

    /// A whole battery snapshot, in the order the bus sends it and the estimate
    /// needs it: `0x101` last, because arriving is what drives a re-estimate.
    /// `peripherals_a` and `motor_a` are the currents as the *display* sees them,
    /// so both are negative while the boat is drawing.
    fn feed_battery(
        data: &mut DisplayData,
        volts: f32,
        peripherals_a: f32,
        motor_a: f32,
        state_of_charge: f32,
    ) {
        let volts = ((volts * 1000.0) as u16).to_le_bytes();
        feed_raw(data, 0x106, &[0, 0, 0, 0, volts[0], volts[1], 0, 0]);

        // Bytes 0-3 are the pack current, which the estimate does not read.
        let peripherals = peripherals_a.to_le_bytes();
        feed_raw(
            data,
            0x100,
            &[
                0,
                0,
                0,
                0,
                peripherals[0],
                peripherals[1],
                peripherals[2],
                peripherals[3],
            ],
        );

        let soc = ((state_of_charge * 100.0) as u16).to_le_bytes();
        feed_raw(data, 0x102, &[soc[0], soc[1], 0, 0, 0, 0, 0, 0]);

        // Positive on the wire; the decoder negates it into the draw the display
        // works in.
        let motor = (-motor_a).to_le_bytes();
        feed_raw(
            data,
            0x101,
            &[0, 0, 0, 0, motor[0], motor[1], motor[2], motor[3]],
        );
    }

    /// `0x107`, which is where the BMS says whether its discharge path is on.
    /// Battery state 4 is OnlyCharge and charge state 3 is FetOn -- the boat
    /// plugged in, taking current and giving none.
    fn feed_discharge_state(data: &mut DisplayData, discharge: u8) {
        feed_raw(data, 0x107, &[20, 20, 20, 20, 20, 4, 3, discharge]);
    }

    /// The boat on the charger: discharge off, a charge current arriving, and no
    /// motor drawing.
    fn feed_charging(data: &mut DisplayData, volts: f32, charge_a: f32, state_of_charge: f32) {
        feed_discharge_state(data, 1); // Idle -- the discharge path is open.

        let volts = ((volts * 1000.0) as u16).to_le_bytes();
        feed_raw(data, 0x106, &[0, 0, 0, 0, volts[0], volts[1], 0, 0]);
        feed_raw(data, 0x100, &[0, 0, 0, 0, 0, 0, 0, 0]);

        let soc = ((state_of_charge * 100.0) as u16).to_le_bytes();
        feed_raw(data, 0x102, &[soc[0], soc[1], 0, 0, 0, 0, 0, 0]);

        let charge = charge_a.to_le_bytes();
        feed_raw(
            data,
            0x101,
            &[charge[0], charge[1], charge[2], charge[3], 0, 0, 0, 0],
        );
    }

    /// On the charger the question turns round. 58.4 V at 20 A is 1168 W in, and
    /// 60 % of a 1450 Wh pack is 870 Wh missing -- about three quarters of an hour.
    #[tokio::test(start_paused = true)]
    async fn a_pack_on_the_charger_counts_up_to_full() {
        let mut data = DisplayData::default();
        feed_charging(&mut data, 58.4, 20.0, 40.0);

        let Some(Endurance::ToFull(seconds)) = data.battery_endurance.get().copied() else {
            panic!("a to-full estimate while the BMS has discharge off");
        };
        assert!(
            (2670..=2695).contains(&seconds),
            "expected ~2681 s (44:41), got {seconds} s"
        );
    }

    /// A full pack still on the charger has nothing left to wait for.
    #[tokio::test(start_paused = true)]
    async fn a_full_pack_on_the_charger_reads_zero() {
        let mut data = DisplayData::default();
        feed_charging(&mut data, 58.4, 20.0, 100.0);
        assert_eq!(
            data.battery_endurance.get().copied(),
            Some(Endurance::ToFull(0))
        );
    }

    /// The direction comes from the BMS, not from which way the current happens to
    /// be flowing: on a sunny reach the panels can out-produce the motor for a few
    /// seconds, and a figure that flipped with the clouds would be useless.
    #[tokio::test(start_paused = true)]
    async fn solar_out_producing_the_motor_still_counts_down() {
        let mut data = DisplayData::default();
        feed_discharge_state(&mut data, 3); // On -- under way.
        feed_battery(&mut data, 58.4, -1.4, -31.2, 87.0);
        // A charge current larger than the draw, as a cloud clearing gives.
        feed_raw(
            &mut data,
            0x101,
            &[0, 0, 0x80, 0x42, 0x9A, 0x99, 0xF9, 0x41],
        );

        assert!(
            matches!(data.battery_endurance.get(), Some(Endurance::ToEmpty(_))),
            "the pack is discharging, whatever the panels are doing"
        );
    }

    /// Silence from the BMS means sailing. A display that has heard no discharge
    /// state must not decide the boat is on the charger.
    #[tokio::test(start_paused = true)]
    async fn no_word_from_the_bms_counts_down() {
        let mut data = DisplayData::default();
        assert!(!data.discharge_is_off());
        feed_battery(&mut data, 58.4, -1.4, -31.2, 87.0);
        assert!(matches!(
            data.battery_endurance.get(),
            Some(Endurance::ToEmpty(_))
        ));
    }

    /// Coming off the charger must not hand the discharge estimate a minute of
    /// remembered charging, nor the other way about -- the two filters are
    /// separate, and each one snaps to its first sample after a gap.
    #[tokio::test(start_paused = true)]
    async fn the_two_directions_do_not_borrow_each_others_memory() {
        let mut data = DisplayData::default();
        feed_charging(&mut data, 58.4, 20.0, 40.0);
        tokio::time::advance(core::time::Duration::from_secs(1)).await;

        // Cast off: discharge on, motor pulling.
        feed_discharge_state(&mut data, 3);
        feed_battery(&mut data, 58.4, -1.4, -31.2, 40.0);

        // 58.4 V x 32.6 A = 1904 W against 40 % of 1450 Wh = 580 Wh: ~1097 s. If
        // the drain filter had inherited the 1168 W charge figure it would read
        // nearer 1800.
        let Some(Endurance::ToEmpty(seconds)) = data.battery_endurance.get().copied() else {
            panic!("a to-empty estimate under way");
        };
        assert!(
            (1085..=1110).contains(&seconds),
            "expected ~1097 s, got {seconds} s -- the filters share memory"
        );
    }

    /// The whole estimate in one reading: 58.4 V across 32.6 A out is 1904 W, and
    /// 87 % of a 1450 Wh pack is 1262 Wh, which is 39 minutes and change.
    #[tokio::test(start_paused = true)]
    async fn endurance_comes_from_the_draw_and_the_charge() {
        let mut data = DisplayData::default();
        feed_battery(&mut data, 58.4, -1.4, -31.2, 87.0);

        let Some(Endurance::ToEmpty(seconds)) = data.battery_endurance.get().copied() else {
            panic!("a to-empty estimate from a full snapshot");
        };
        assert!(
            (2375..=2395).contains(&seconds),
            "expected ~2385 s (39:45), got {seconds} s"
        );
    }

    /// The point of the filter. A throttle burst multiplies the draw, and an
    /// unfiltered estimate would follow it straight down -- the helm would watch
    /// the endurance halve every time they opened the throttle and learn to ignore
    /// the figure. Five seconds into a burst it should barely have moved.
    #[tokio::test(start_paused = true)]
    async fn a_throttle_burst_does_not_halve_the_estimate() {
        let mut data = DisplayData::default();
        feed_battery(&mut data, 58.4, -1.4, -31.2, 87.0);
        let Some(Endurance::ToEmpty(settled)) = data.battery_endurance.get().copied() else {
            panic!("settled");
        };

        for _ in 0..5 {
            tokio::time::advance(core::time::Duration::from_secs(1)).await;
            feed_battery(&mut data, 58.4, -1.4, -80.0, 87.0);
        }
        let Some(Endurance::ToEmpty(burst)) = data.battery_endurance.get().copied() else {
            panic!("in a burst");
        };

        assert!(
            burst < settled,
            "the estimate has to move towards the heavier draw eventually"
        );
        assert!(
            burst > settled * 85 / 100,
            "5 s into a burst the estimate dropped from {settled} s to {burst} s, \
             which is the unfiltered reading, not a smoothed one"
        );
    }

    /// Tied up with nothing but the hotel load, the division runs away towards
    /// days. Dashes are the honest reading: not "forever", but "not a time".
    #[tokio::test(start_paused = true)]
    async fn a_boat_at_rest_gets_no_estimate() {
        let mut data = DisplayData::default();
        feed_battery(&mut data, 58.4, -0.05, 0.0, 87.0);
        assert_eq!(data.battery_endurance.get(), None);
    }

    /// A frame arriving before the readings it needs must not publish half an
    /// estimate -- or panic on the missing ones.
    #[tokio::test(start_paused = true)]
    async fn a_draw_with_no_voltage_yet_estimates_nothing() {
        let mut data = DisplayData::default();
        feed_raw(&mut data, 0x101, &[0, 0, 0, 0, 0, 0, 0xFA, 0x41]); // 31.2 A out
        assert_eq!(data.battery_endurance.get(), None);
    }

    /// The estimate is telemetry, not configuration: a bus that goes quiet leaves
    /// the helm with dashes rather than a figure from a situation that has passed.
    #[tokio::test(start_paused = true)]
    async fn the_estimate_ages_out_with_the_bus() {
        let mut data = DisplayData::default();
        feed_battery(&mut data, 58.4, -1.4, -31.2, 87.0);
        assert!(data.battery_endurance.get().is_some());

        tokio::time::advance(core::time::Duration::from_secs(6)).await;
        assert_eq!(data.battery_endurance.get(), None);
    }

    /// Index 16 is `PTCH_RATE_P`, which the screen draws at Pitch row 1.
    #[test]
    fn a_readback_lands_in_its_cell() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 2.93);
        assert_eq!(
            data.foiling.pitch[0].get().copied(),
            Some(Reading::One(2.93))
        );
        // The roll side of the same row is a different parameter and untouched.
        assert_eq!(data.foiling.roll[0].get().copied(), None);
        feed(&mut data, 1, 0, 0.33);
        assert_eq!(
            data.foiling.roll[0].get().copied(),
            Some(Reading::One(0.33))
        );
    }

    /// A parameter that does not exist yet is sent with a value of zero, which must
    /// not be drawn: `HYD_*` reads back unavailable until `hydrofoils.lua` has
    /// created them, and a gain of 0 would look like a deliberate setting.
    #[test]
    fn an_unavailable_parameter_leaves_the_cell_empty() {
        let mut data = DisplayData::default();
        feed(&mut data, 32, 4, 0.0);
        assert_eq!(data.foiling.height[0].get().copied(), None);
        // Locked still carries the live value, so that one lands.
        feed(&mut data, 32, 5, 1200.0);
        assert_eq!(
            data.foiling.height[0].get().copied(),
            Some(Reading::One(1200.0))
        );
    }

    /// The height command clamps are two parameters in one cell, so neither half
    /// may overwrite the other, and nothing is drawn until both have arrived.
    #[test]
    fn a_pair_needs_both_halves() {
        let mut data = DisplayData::default();
        feed(&mut data, 52, 0, 5.0); // HYD_CMDMAX
        assert_eq!(
            data.foiling.height[5].get().copied(),
            None,
            "half a pair cannot be drawn honestly"
        );
        feed(&mut data, 53, 0, -8.0); // HYD_CMDMIN
        assert_eq!(
            data.foiling.height[5].get().copied(),
            Some(Reading::UpDown(5.0, -8.0))
        );
        // A later update to one half keeps the other.
        feed(&mut data, 52, 0, 4.5);
        assert_eq!(
            data.foiling.height[5].get().copied(),
            Some(Reading::UpDown(4.5, -8.0))
        );
    }

    /// The pitch rate max is a pair too, and reads as one number while its halves
    /// agree -- the renderer collapses it, so both are always stored.
    #[test]
    fn a_symmetric_pair_is_still_stored_as_two_halves() {
        let mut data = DisplayData::default();
        feed(&mut data, 22, 0, 60.0);
        feed(&mut data, 23, 0, 60.0);
        assert_eq!(
            data.foiling.pitch[6].get().copied(),
            Some(Reading::UpDown(60.0, 60.0))
        );
    }

    /// A run of steps on one cell reads as a single movement: `from` is the value
    /// the burst started at, not the previous keypress.
    #[test]
    fn an_edit_burst_holds_its_starting_value() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 2.10);
        assert!(
            data.foiling.last_edit().is_none(),
            "the first read is not an edit"
        );

        feed(&mut data, 16, 0, 2.15);
        feed(&mut data, 16, 0, 2.20);
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!((edit.column, edit.row), (FoilColumn::Pitch, 1));
        assert!(
            (edit.from - 2.10).abs() < 1e-6,
            "from held across the burst"
        );
        assert!((edit.to - 2.20).abs() < 1e-6);

        // A different cell starts a new burst.
        feed(&mut data, 1, 0, 0.33);
        feed(&mut data, 1, 0, 0.35);
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!((edit.column, edit.row), (FoilColumn::Roll, 1));
        assert!((edit.from - 0.33).abs() < 1e-6);
    }

    /// A write the flight controller clamped is an edit even when the number does
    /// not move. Pressing `+` against a parameter's maximum changes nothing on
    /// screen, so the status line is the only thing that can say why -- and it only
    /// gets the chance if the clamped frame is recorded.
    #[test]
    fn a_clamped_write_is_an_edit_even_when_nothing_moves() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 7.98); // PTCH_RATE_P, an ordinary read-back
        assert!(data.foiling.last_edit().is_none(), "a read is not an edit");

        // Asked for more than 8, clamped to the parameter's maximum. The value did
        // move, so which bound it hit is not in doubt.
        feed(&mut data, 16, 2, 8.0);
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!(edit.clamped, Some(FoilLimit::Max));
        assert!((edit.from - 7.98).abs() < 1e-6);
        assert!((edit.to - 8.0).abs() < 1e-6);

        // Held against the stop: nothing moves now, and the bound is remembered
        // from the press that reached it rather than being guessed again.
        feed(&mut data, 16, 2, 8.0);
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!(edit.clamped, Some(FoilLimit::Max));
        assert!((edit.from - 7.98).abs() < 1e-6, "still one burst");
    }

    /// What a walk into a stop actually looks like on the bus: the step that lands
    /// exactly on the bound reads back unclamped, and the *next* press is the
    /// clamped one that moves nothing. The bound comes from the direction the burst
    /// was already going.
    #[test]
    fn a_clamp_takes_its_bound_from_the_walk_that_reached_it() {
        let mut data = DisplayData::default();
        feed(&mut data, 8, 0, 15.0); // ROLL_LIMIT_DEG, whose maximum is 20
        feed(&mut data, 8, 0, 20.0); // a step that landed on it, so not clamped
        feed(&mut data, 8, 2, 20.0); // and the press that goes nowhere
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!(edit.clamped, Some(FoilLimit::Max));
        assert!(
            (edit.from - 15.0).abs() < 1e-6,
            "one burst, from where it began"
        );
    }

    /// Pressing against a cell that was already on its limit before the cursor
    /// arrived: the value does not move and there is no earlier press to take a
    /// direction from, so the bound is genuinely unknown and is not invented.
    #[test]
    fn a_clamp_with_no_history_does_not_name_the_bound() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 8.0);
        feed(&mut data, 16, 2, 8.0);
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!(edit.clamped, Some(FoilLimit::Unknown));
        assert!((edit.from - edit.to).abs() < f32::EPSILON);
    }

    /// The same value arrives repeatedly -- the tuner re-requests the selected cell
    /// while the cursor rests on it, and the boot dump re-sends every parameter.
    /// Those re-reads must not count as edits, or the status line would be replaced
    /// by an empty movement whenever the helm stops moving.
    #[test]
    fn an_unchanged_readback_leaves_the_status_line_alone() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 4.05);
        feed(&mut data, 16, 0, 4.07);
        let edit = data.foiling.last_edit().expect("an edit");
        feed(&mut data, 16, 0, 4.07);
        assert_eq!(
            data.foiling.last_edit(),
            Some(edit),
            "the dump changed nothing"
        );
        assert_eq!(edit.clamped, None);
    }

    /// The slot column is drawn from `0x263`, which the datalogger repeats: the
    /// display keeps no configuration state of its own, so this is the only thing
    /// that decides what a slot is labelled.
    #[test]
    fn a_slot_message_labels_its_slot() {
        let mut data = DisplayData::default();
        assert_eq!(
            data.foiling.slots[3].get(),
            None,
            "empty until told otherwise"
        );

        feed_raw(&mut data, 0x263, &[4, 1, 14, 32]);
        assert_eq!(data.foiling.slots[3].get().copied(), Some(Some((14, 32))));
        // Slots are numbered as labelled, so slot 4 is the fourth cell and nothing
        // else moved.
        assert_eq!(data.foiling.slots[4].get(), None);

        // Stored without a fix: something is there, but there is no time to show.
        feed_raw(&mut data, 0x263, &[4, 2]);
        assert_eq!(data.foiling.slots[3].get().copied(), Some(None));

        // Wiped, which has to read as empty at once rather than after the staleness
        // timeout.
        feed_raw(&mut data, 0x263, &[4, 0]);
        assert_eq!(data.foiling.slots[3].get(), None);

        // Slot 0 does not exist -- the column is 1..9 -- and must not wrap onto the
        // ninth.
        feed_raw(&mut data, 0x263, &[0, 1, 9, 15]);
        assert!(data.foiling.slots.iter().all(|slot| slot.get().is_none()));
    }

    /// Restoring a slot writes the whole table, so the read-backs that follow are
    /// all parameters that changed. The line has to survive them, or the one thing
    /// the helm needs to see -- that the restore happened -- is gone before the
    /// panel has refreshed once.
    #[test]
    fn a_restore_holds_the_line_against_the_readbacks_it_causes() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 4.05);
        feed_raw(&mut data, 0x264, &[2, 4, 1, 14, 32]); // config 4 restored

        feed(&mut data, 16, 0, 2.10); // one of the fifty writes landing
        assert_eq!(
            data.foiling.last_event,
            Some(FoilEvent::Slot(FoilSlotEvent {
                action: FoilConfigAction::Restored,
                slot: 4,
                time: Some((14, 32)),
            })),
            "the restore still owns the line"
        );
        // The values themselves are not held back, only the sentence.
        assert_eq!(
            data.foiling.pitch[0].get().copied(),
            Some(Reading::One(2.10))
        );
        assert!(data.foiling.last_edit().is_none());
    }

    /// An action from a later protocol tells the display nothing it can put into
    /// words, so it leaves the line as it found it rather than blanking it.
    #[test]
    fn an_unknown_slot_action_is_ignored() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 4.05);
        feed(&mut data, 16, 0, 4.07);
        feed_raw(&mut data, 0x264, &[9, 4, 0]);
        assert!(data.foiling.last_edit().is_some(), "the edit line stands");
    }

    /// The tuner re-requests the selected cell once the cursor settles, so the most
    /// recent read-back is where it is.
    #[test]
    fn the_cursor_follows_the_last_readback() {
        let mut data = DisplayData::default();
        feed(&mut data, 44, 0, 4.0); // TRN_RATE -> Right row 5
        assert_eq!(
            data.foiling.cursor.get().copied(),
            Some(FoilCursor {
                column: FoilColumn::Right,
                row: 5
            })
        );
    }

    /// The cursor is inferred from the last read-back, which is what the protocol
    /// offers -- there is no cursor frame. During the connect-time dump that walks
    /// the whole table and ends up wherever the dump ended, which is wrong; the
    /// tuner's re-request of the selected cell then corrects it.
    ///
    /// This is the weakest part of the wire contract and the test says so: an
    /// explicit cursor frame would remove the transient entirely.
    #[test]
    fn the_cursor_settles_after_a_dump_corrects_it() {
        let mut data = DisplayData::default();
        // A dump, in index order, ending on SCR_USER4 -> Right row 11.
        for index in [16, 17, 32, 40, 48, 49, 50, 51] {
            feed(&mut data, index, 0, 1.0);
        }
        assert_eq!(
            data.foiling.cursor.get().copied(),
            Some(FoilCursor {
                column: FoilColumn::Right,
                row: 11
            }),
            "mid-dump the cursor sits wherever the dump ended"
        );

        // The tuner re-requests the selected cell once the cursor settles.
        feed(&mut data, 16, 0, 1.0);
        assert_eq!(
            data.foiling.cursor.get().copied(),
            Some(FoilCursor {
                column: FoilColumn::Pitch,
                row: 1
            }),
            "the re-request puts it where the operator actually is"
        );
    }

    /// Every index in the tuner's table has to land somewhere drawable, or a
    /// parameter would arrive and vanish.
    #[test]
    fn every_tuner_index_maps_to_a_cell() {
        let indices = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
            28, 29, 30, 31, 32, 33, 34, 35, 36, 39, 40, 41, 42, 43, 44, 45, 48, 49, 50, 51, 52, 53,
            54, 55, 56, 57,
        ];
        assert_eq!(
            indices.len(),
            50,
            "foil_tune.lua PROTO_VERSION 9 has 50 the screen draws"
        );
        let mut data = DisplayData::default();
        for index in indices {
            let cell = render::foiling::cell_for_index(index);
            assert!(cell.is_some(), "index {index} has no cell");
            let (column, row, _) = cell.unwrap();
            // Reachable in the data as well as on the map.
            feed(&mut data, index, 0, 1.0);
            assert!(
                data.foiling.values_mut(column, row).is_some(),
                "index {index} maps to {column:?} row {row}, which has no slot"
            );
        }
        // Retired and unused indices must not resolve.
        for index in [0, 13, 14, 15, 37, 38, 46, 47, 58, 59, 200, 0xFD] {
            assert!(
                render::foiling::cell_for_index(index).is_none(),
                "index {index} should not map anywhere"
            );
        }
    }

    /// A tuning parameter outlives the telemetry timeout, because nothing on the
    /// bus would ever refresh it. `foil_tune.lua` dumps the table once ~5 s after
    /// the flight controller boots and is event-driven after that, so expiring
    /// these cells sent the whole screen to dashes seconds after the only burst
    /// it was ever going to get.
    #[tokio::test(start_paused = true)]
    async fn a_parameter_is_held_past_the_telemetry_timeout() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 2.93);
        tokio::time::advance(core::time::Duration::from_secs(600)).await;
        assert_eq!(
            data.foiling.pitch[0].get().copied(),
            Some(Reading::One(2.93)),
            "a gain the flight controller acknowledged is still the gain in effect"
        );
        // Never heard is still distinguishable from held: only ageing out is gone.
        assert_eq!(data.foiling.roll[0].get().copied(), None);
    }

    /// The slot column is the other way round: the datalogger republishes all nine
    /// at ~1 Hz, so one that stops arriving really is unknown -- and a slot wiped
    /// on the tuner has to stop being labelled here.
    #[tokio::test(start_paused = true)]
    async fn a_config_slot_still_expires() {
        let mut data = DisplayData::default();
        feed_raw(&mut data, 0x263, &[4, 1, 14, 32]);
        assert_eq!(data.foiling.slots[3].get().copied(), Some(Some((14, 32))));
        tokio::time::advance(core::time::Duration::from_secs(6)).await;
        assert_eq!(data.foiling.slots[3].get(), None, "the column is telemetry");
    }

    /// The status line's `from` is read back out of the cell, so latching has to
    /// reach that path too: after a long quiet spell the next edit must still
    /// report the movement rather than finding an empty cell and saying nothing.
    #[tokio::test(start_paused = true)]
    async fn an_edit_after_a_long_quiet_spell_still_reports_its_movement() {
        let mut data = DisplayData::default();
        feed(&mut data, 16, 0, 2.00);
        tokio::time::advance(core::time::Duration::from_secs(600)).await;
        feed(&mut data, 16, 0, 2.50);
        let edit = data.foiling.last_edit().expect("an edit");
        assert_eq!((edit.from, edit.to), (2.00, 2.50));
    }

    /// Which semantics a cell has is the compiler's job, not a convention every
    /// call site has to remember. This fails to build if a parameter array is
    /// given telemetry semantics again.
    #[test]
    fn parameter_cells_latch_and_the_rest_do_not() {
        let data = FoilingData::default();
        let _: &Latched<Reading> = &data.pitch[0];
        let _: &Latched<Reading> = &data.roll[0];
        let _: &Latched<Reading> = &data.height[0];
        let _: &Latched<Reading> = &data.rear[0];
        let _: &Latched<Reading> = &data.turn[0];
        let _: &Latched<Reading> = &data.mode[0];
        let _: &Latched<Reading> = &data.global[0];
        let _: &DisplayValue<Option<(u8, u8)>> = &data.slots[0];
        let _: &DisplayValue<FoilCursor> = &data.cursor;
    }

    /// Where a read-back is *stored* has to be where the screen *draws* it. The
    /// map and the slot existing is not enough: the rear block was written one
    /// slot past the cell it is drawn in for a day, so `RKP` showed dashes and the
    /// three rows under it showed their neighbour's gain.
    #[test]
    fn a_readback_is_drawn_in_the_cell_it_lands_in() {
        for (nth, index) in [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 17, 18, 19, 20, 21, 24, 27, 28, 29, 30, 31,
            32, 33, 34, 35, 36, 39, 40, 41, 42, 43, 44, 45, 48, 49, 50, 51, 54, 55, 56, 57,
        ]
        .into_iter()
        .enumerate()
        {
            // A distinct value per index, so a cell holding its neighbour's
            // reading cannot pass by coincidence.
            let sent = 1.0 + nth as f32;
            let mut data = DisplayData::default();
            feed(&mut data, index, 0, sent);

            let (column, row, _) = render::foiling::cell_for_index(index).expect("a cell");
            let drawn = render::foiling::drawn_reading(&data.foiling, column, row);
            assert_eq!(
                drawn,
                Some(Reading::One(sent)),
                "index {index} is stored at {column:?} row {row} but drawn elsewhere"
            );
        }
    }
}
