//! Shared drawing primitives for every screen layout.
//!
//! This module owns everything a layout needs but does not choose: the `Cell`
//! geometry model, the fonts and their measured metrics, the widgets built from
//! them, and the value formatters. It places nothing itself.
//!
//! Each screen is a submodule that owns its own positions and its own
//! compile-time layout assertions, and exposes one `fn(&mut D, &DisplayData)`:
//!
//! - [`dashboard`] -- the helm screen: speed, power, temperatures, times.
//! - [`foiling`] -- the foiling trim and tuning parameters.
//!
//! Font metrics are hardcoded here so layout arithmetic can be `const`, and
//! `font_metrics_match_their_consts` fails if a rebuilt font drifts from them.

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, MonoTextStyle, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use heapless::String;
use u8g2_fonts::{
    types::{FontColor, HorizontalAlignment, VerticalPosition},
    FontRenderer,
};

pub mod dashboard;
pub mod foiling;

pub const DISPLAY_WIDTH: u32 = 792;
pub const DISPLAY_HEIGHT: u32 = 272;

/// A rectangular region in absolute screen pixels.
///
/// Layout is expressed by subdividing cells rather than by absolute magic
/// numbers, so a cell's contents are positioned relative to its own edges and
/// every split retunes when its parent changes. `embedded-graphics` has no
/// layout engine -- it draws at absolute coordinates -- so this is the whole of
/// the geometry model.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Cell {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Cell {
    const ZERO: Self = Self {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    const fn right(&self) -> i32 {
        self.x + self.w
    }

    const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    const fn center_x(&self) -> i32 {
        self.x + self.w / 2
    }

    /// Split into columns proportional to `weights`. The last column absorbs
    /// the rounding remainder, so the parts always tile the parent exactly.
    const fn cols<const N: usize>(&self, weights: [i32; N]) -> [Self; N] {
        let mut total = 0;
        let mut i = 0;
        while i < N {
            total += weights[i];
            i += 1;
        }

        let mut out = [Self::ZERO; N];
        let mut x = self.x;
        let mut i = 0;
        while i < N {
            let w = if i + 1 == N {
                self.right() - x
            } else {
                self.w * weights[i] / total
            };
            out[i] = Self {
                x,
                y: self.y,
                w,
                h: self.h,
            };
            x += w;
            i += 1;
        }
        out
    }

    /// Split into `N` equal-height rows, the last absorbing the remainder.
    const fn rows<const N: usize>(&self) -> [Self; N] {
        let mut out = [Self::ZERO; N];
        let pitch = self.h / N as i32;
        let mut i = 0;
        while i < N {
            let h = if i + 1 == N {
                self.bottom() - (self.y + pitch * i as i32)
            } else {
                pitch
            };
            out[i] = Self {
                x: self.x,
                y: self.y + pitch * i as i32,
                w: self.w,
                h,
            };
            i += 1;
        }
        out
    }

    /// Shrink by `dx` on the left and right, `dy` on the top and bottom.
    const fn inset(&self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: self.w - 2 * dx,
            h: self.h - 2 * dy,
        }
    }

    /// Centre y of each item in a vertically centred stack of `heights`
    /// separated by `gap`. Used to hang a label off a big value without
    /// hand-tuning either position.
    const fn stack_centers<const N: usize>(&self, heights: [i32; N], gap: i32) -> [i32; N] {
        let mut total = gap * (N as i32 - 1);
        let mut i = 0;
        while i < N {
            total += heights[i];
            i += 1;
        }

        let mut out = [0; N];
        let mut y = self.y + (self.h - total) / 2;
        let mut i = 0;
        while i < N {
            out[i] = y + heights[i] / 2;
            y += heights[i] + gap;
            i += 1;
        }
        out
    }
}

const SCREEN: Cell = Cell {
    x: 0,
    y: 0,
    w: DISPLAY_WIDTH as i32,
    h: DISPLAY_HEIGHT as i32,
};
/// Height reserved across the very top for the 4x6 stamp line: the IP address in
/// the left corner and the build stamp over the centre column. `FONT_4X6` at
/// `VERSION_BASELINE_Y` reaches down to y=6, so this keeps the headlines' ink
/// clear of it.
const STAMP_BAND_H: i32 = 8;
/// Advance of '%' in `FONT_BIG`, pinned by `font_metrics_match_the_layout`.
const BIG_PCT_W: i32 = 63;
/// Advance of "°C" in `FONT_SMALL`, pinned by `font_metrics_match_the_layout`.
const SMALL_DEG_C_W: i32 = 21;
/// Gap between a temperature and its degree sign.
const VALUE_UNIT_GAP: i32 = 4;
/// Gap between a temperature and its label. Independent of `STACK_LABEL_GAP` --
/// this one separates a value from a label, not two label lines.
const LABEL_GAP: i32 = 7;
/// Metrics of `FONT_NET`, pinned by `font_metrics_match_the_layout`.
const NET_DIGIT_H: i32 = 58;
const NET_DIGIT_W: i32 = 48;
/// Advance of '-' in `FONT_NET`, pinned by `font_metrics_match_the_layout`.
const NET_MINUS_W: i32 = 32;
/// Metrics of `FONT_MID`, pinned by `font_metrics_match_the_layout`.
const MID_DIGIT_H: i32 = 29;
const MID_DIGIT_W: i32 = 24;
const MID_MINUS_W: i32 = 16;
/// Advance of "W" in `FONT_SMALL`, pinned by `font_metrics_match_the_layout`.
const SMALL_W_W: i32 = 18;
/// Vertical gap between the two lines of a stacked unit/label block.
const STACK_LABEL_GAP: i32 = 6;
/// The speed's whole numbers are bitmaps, not a font: u8g2-fonts cannot decode a
/// bit field wider than 7, which caps a font at a 63px advance and so at 76px
/// digits for this face. `support/ttf-digits-to-raw.py` rasterises them instead,
/// every glyph in a cell of the same size on a common baseline.
///
/// Uniform cells mean the "--" placeholder is exactly as wide as a real "14", so
/// the block never shifts. `speed_glyphs_match_their_cells` checks the blob.
const SPEED_GLYPH_ORDER: &str = "0123456789-";
const SPEED_GLYPHS: &[u8] = include_bytes!("../../assets/speed115.raw");
const SPEED_GLYPH_ROW_BYTES: usize = (SPEED_DIGIT_W as usize).div_ceil(8);
const SPEED_GLYPH_BYTES: usize = SPEED_GLYPH_ROW_BYTES * SPEED_DIGIT_H as usize;
/// Cell size of those bitmaps, and the metrics of `FONT_SMALL`. Hardcoded so the
/// speed layout is const; the tests fail if either drifts.
const SPEED_DIGIT_H: i32 = 115;
const SPEED_DIGIT_W: i32 = 95;
const SPEED_DOT_W: i32 = 10;
const SMALL_CAP_H: i32 = 14;
/// Blank columns inside a digit cell, and inside `FONT_BIG`'s glyphs. Subtracted
/// out so `SPEED_DOT_GAP` means what it says. The digit figure is the *narrowest*
/// bearing of any digit -- '1' and '4' reach furthest right -- so the gap holds for
/// the worst case rather than the average.
const SPEED_DIGIT_BEARING: i32 = 6;
const BIG_DOT_BEARING: i32 = 5;
const BIG_DIGIT_BEARING: i32 = 3;
/// Metrics of `FONT_BIG`, which draws the right column and both the speed's dot
/// and its tenth. Pinned by `font_metrics_match_the_layout`.
const BIG_DIGIT_H: i32 = 49;
const BIG_DIGIT_W: i32 = 40;
/// The build stamp sits at the very top of the centre column, above the speed.
/// `FONT_4X6` is 6px tall, so a baseline here puts it flush with the screen edge.
/// The IP address shares this baseline in the top-left corner.
const VERSION_BASELINE_Y: i32 = 6;
/// Colon advance of `FONT_MID`, pinned by `font_metrics_match_the_layout`. All
/// three times share that font, so they share one set of metrics.
const MID_COLON_W: i32 = 12;
/// Geometry of one HH:MM:SS block.
///
/// The colons are drawn at fixed positions with each two-character group centred
/// in its own slot, so the block holds still when dashes stand in for missing
/// data -- a dash is narrower than a digit, so a plain string would shrink and
/// shift the whole time.
#[derive(Copy, Clone)]
struct TimeMetrics {
    digit_w: i32,
    colon_w: i32,
}

impl TimeMetrics {
    /// Two digits.
    const fn group_w(&self) -> i32 {
        2 * self.digit_w
    }

    const fn total_w(&self) -> i32 {
        3 * self.group_w() + 2 * self.colon_w
    }

    /// Left edge of group `index` (0..3) in a block starting at `left`.
    const fn group_x(&self, left: i32, index: i32) -> i32 {
        left + index * (self.group_w() + self.colon_w)
    }

    /// Left edge of the colon after group `index` (0..2).
    const fn colon_x(&self, left: i32, index: i32) -> i32 {
        self.group_x(left, index) + self.group_w()
    }
}

const TIME_METRICS: TimeMetrics = TimeMetrics {
    digit_w: MID_DIGIT_W,
    colon_w: MID_COLON_W,
};
/// Gap between a time and its label block.
const TIME_LABEL_GAP: i32 = 8;
/// Gap between a value and the label block beside it.
const BLOCK_GAP: i32 = 10;
// IBM Plex Sans Medium (wght 500), rasterised from the Google Fonts variable TTF by
// `support/build-fonts.py` (see fonts/README.md). u8g2 ships no Plex, so these
// are our own blobs; the crate's `Font` trait is public, which is all it takes.
//
// Plex Sans has tabular figures -- all ten digits share one advance -- so values
// don't shift width as they change without forcing a monospace build, and '-',
// '.' and ':' keep their natural narrow widths. The stock Inconsolata _mn fonts
// were monospace, which gave punctuation a full digit cell; in a proportional
// face that reads as "- 2019" and "17: 42: 23", so these are proportional.
//
// Subset per font: the value fonts carry only " -.:0123456789", the label font
// printable ASCII plus U+00B0 for "°C".
macro_rules! plex_font {
    ($name:ident, $file:literal) => {
        struct $name;
        impl u8g2_fonts::Font for $name {
            const DATA: &'static [u8] = include_bytes!(concat!("../../fonts/", $file));
        }
    };
}

plex_font!(PlexNet58, "plex_net58_tn.u8g2font");
plex_font!(PlexBig49, "plex_big49_tn.u8g2font");
plex_font!(PlexMid30, "plex_mid30_tn.u8g2font");
plex_font!(PlexSmall14, "plex_small14_tf.u8g2font");
plex_font!(PlexSmall12, "plex_small12_tf.u8g2font");
plex_font!(PlexSemi12, "plex_semi12_tf.u8g2font");

/// Net power, the left column's headline.
const FONT_NET: FontRenderer = FontRenderer::new::<PlexNet58>();
/// The right column's values.
const FONT_BIG: FontRenderer = FontRenderer::new::<PlexBig49>();
const FONT_MID: FontRenderer = FontRenderer::new::<PlexMid30>();
const FONT_SMALL: FontRenderer = FontRenderer::new::<PlexSmall14>();
/// Two points smaller, for the foiling screen: four tables abreast need the width
/// back, and it is a screen read leaning in rather than at a glance.
const FONT_TINY: FontRenderer = FontRenderer::new::<PlexSmall12>();
/// The foiling screen's table headings: the same 12px cap as [`FONT_TINY`] in a
/// heavier weight, so a heading is told from the parameter labels under it without
/// moving a single row. Only the weight differs, which is the one font property
/// this layout does not derive anything from.
const FONT_HEADING: FontRenderer = FontRenderer::new::<PlexSemi12>();
/// Cap height of [`FONT_TINY`] and [`FONT_HEADING`], pinned by
/// `font_metrics_match_their_consts`.
const TINY_CAP_H: i32 = 12;

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

/// Draw `text` vertically centred on `center_y`, aligned horizontally against
/// `anchor_x`. The three alignments cover every element on the screen: values
/// are right-aligned so digits line up, the speed is centred in its column, and
/// labels sit left-aligned after the value they belong to.
fn draw_text<D, C>(
    display: &mut D,
    font: &FontRenderer,
    align: HorizontalAlignment,
    anchor_x: i32,
    center_y: i32,
    text: &str,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    font.render_aligned(
        text,
        Point::new(anchor_x, center_y),
        VerticalPosition::Center,
        align,
        FontColor::Transparent(BinaryColor::Off.into()),
        display,
    )
    .map_err(map_font_err)?;
    Ok(())
}

/// A value with its unit beside it and its name underneath, all ending on `right`.
///
/// The shape both the temperature grid and the power in/out rows use, so the two
/// columns read the same way: digits right-aligned, unit sitting on the digits'
/// bottom edge rather than centred on them, label right-aligned below.
#[allow(clippy::too_many_arguments)]
fn draw_reading<D, C>(
    display: &mut D,
    buf: &mut String<16>,
    right: i32,
    value: Option<f32>,
    unit: &str,
    unit_w: i32,
    label: &str,
    value_y: i32,
    label_y: i32,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let unit_x = right - unit_w;

    draw_text(
        display,
        &FONT_MID,
        HorizontalAlignment::Right,
        unit_x - VALUE_UNIT_GAP,
        value_y,
        fmt_f32(buf, value, 0, "--"),
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Left,
        unit_x,
        value_y + MID_DIGIT_H / 2 - SMALL_CAP_H / 2,
        unit,
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Right,
        right,
        label_y,
        label,
    )
}

/// One cell of the temperature grid.
fn draw_temperature<D, C>(
    display: &mut D,
    cell: Cell,
    buf: &mut String<16>,
    value: Option<f32>,
    label: &str,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let [value_y, label_y] = cell.stack_centers([MID_DIGIT_H, SMALL_CAP_H], LABEL_GAP);
    draw_reading(
        display,
        buf,
        cell.right(),
        value,
        "°C",
        SMALL_DEG_C_W,
        label,
        value_y,
        label_y,
    )
}

/// A two-line label block -- unit over qualifier -- left-aligned at `left_x` and
/// vertically centred on `center_y`. Used for the "W" over "in"/"out" blocks.
fn draw_stacked_label<D, C>(
    display: &mut D,
    left_x: i32,
    center_y: i32,
    unit: &str,
    label: &str,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let block = Cell {
        x: left_x,
        y: center_y - SMALL_CAP_H,
        w: 0,
        h: 2 * SMALL_CAP_H,
    };
    let [unit_y, label_y] = block.stack_centers([SMALL_CAP_H, SMALL_CAP_H], STACK_LABEL_GAP);

    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Left,
        left_x,
        unit_y,
        unit,
    )?;
    draw_text(
        display,
        &FONT_SMALL,
        HorizontalAlignment::Left,
        left_x,
        label_y,
        label,
    )
}

/// A time with its label hung off the right, filling one bottom-row cell.
fn draw_time<D, C>(
    display: &mut D,
    font: &FontRenderer,
    metrics: TimeMetrics,
    left: i32,
    center_y: i32,
    groups: [&str; 3],
    label: Option<([&str; 2], i32)>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    // Each group centred in its own slot, so a narrower "--" stays put.
    for (index, group) in groups.iter().enumerate() {
        let index = index as i32;
        draw_text(
            display,
            font,
            HorizontalAlignment::Center,
            metrics.group_x(left, index) + metrics.digit_w,
            center_y,
            group,
        )?;
    }
    for index in 0..2 {
        draw_text(
            display,
            font,
            HorizontalAlignment::Left,
            metrics.colon_x(left, index),
            center_y,
            ":",
        )?;
    }

    if let Some((lines, _)) = label {
        draw_stacked_label(
            display,
            left + metrics.total_w() + TIME_LABEL_GAP,
            center_y,
            lines[0],
            lines[1],
        )?;
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
/// Format hours/minutes/seconds as three two-character groups, or dashes.
///
/// The colons are not included: `draw_time` places them at fixed positions so the
/// block does not move when dashes replace digits.
fn fmt_hms_groups(buf: &mut String<16>, hms: Option<(u8, u8, u8)>) -> [&str; 3] {
    match hms {
        Some((h, m, s)) => {
            buf.clear();
            write!(buf, "{h:02}{m:02}{s:02}").unwrap();
            let text = buf.as_str();
            [&text[0..2], &text[2..4], &text[4..6]]
        }
        None => ["--", "--", "--"],
    }
}

/// One line of the 4x6 stamp band across the top of the screen.
///
/// The band is the only place a proportional-font layout drops to a mono font:
/// it carries diagnostics (build stamp, IP) rather than readings, so it is sized
/// for density instead of legibility at distance. `STAMP_BAND_H` reserves the
/// room a layout must leave under it.
fn draw_stamp<D, C>(display: &mut D, x: i32, align: Alignment, text: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    let style: MonoTextStyle<'_, C> = MonoTextStyleBuilder::new()
        .font(&FONT_4X6)
        .text_color(BinaryColor::Off.into())
        .background_color(BinaryColor::On.into())
        .build();

    Text::with_alignment(text, Point::new(x, VERSION_BASELINE_Y), style, align).draw(display)?;
    Ok(())
}

/// Width of `text` as `font` will render it. Layout tests in every submodule
/// measure against this, so it lives with the fonts rather than in one of them.
#[cfg(test)]
fn width(font: &FontRenderer, text: &str) -> i32 {
    font.get_rendered_dimensions(text, Point::zero(), VerticalPosition::Center)
        .unwrap()
        .advance
        .x
}

/// Height of a digit as `font` will render it.
#[cfg(test)]
fn digit_height(font: &FontRenderer) -> i32 {
    font.get_rendered_dimensions("0", Point::zero(), VerticalPosition::Center)
        .unwrap()
        .bounding_box
        .expect("digit renders")
        .size
        .height as i32
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
    fn fmt_hms_groups_splits_and_dashes() {
        let mut buf: String<16> = String::new();
        // Colons are excluded -- `draw_time` places them itself.
        assert_eq!(
            fmt_hms_groups(&mut buf, Some((23, 55, 1))),
            ["23", "55", "01"]
        );
        assert_eq!(fmt_hms_groups(&mut buf, None), ["--", "--", "--"]);
    }

    /// Every metric the layouts do const arithmetic with, checked against the
    /// fonts as built. A rebuilt or resubsetted font that shifts an advance
    /// fails here rather than silently clipping a value on the panel.
    #[test]
    fn font_metrics_match_their_consts() {
        // The dot is drawn in FONT_BIG, matching the tenth beside it.
        let dot = FONT_BIG
            .get_rendered_dimensions(".", Point::zero(), VerticalPosition::Center)
            .unwrap()
            .bounding_box
            .expect("dot renders")
            .size
            .width as i32;
        assert!(
            dot <= SPEED_DOT_W,
            "dot ink is {dot}px, wider than the {SPEED_DOT_W}px reserved for it"
        );

        for (name, font, expected) in [
            ("FONT_SMALL", &FONT_SMALL, SMALL_CAP_H),
            ("FONT_TINY", &FONT_TINY, TINY_CAP_H),
            // The heading font shares the row grid with FONT_TINY, so it has to
            // share its cap height too -- a weight that came out taller would push
            // the top row off the screen edge.
            ("FONT_HEADING", &FONT_HEADING, TINY_CAP_H),
        ] {
            let cap = font
                .get_rendered_dimensions("T", Point::zero(), VerticalPosition::Center)
                .unwrap()
                .bounding_box
                .expect("cap renders")
                .size
                .height as i32;
            assert_eq!(cap, expected, "{name} cap height changed");
        }

        // Glyph advances that position a neighbour: the minus that widens a
        // reserved field, the percent and unit that hang off a value, and the
        // colon the time blocks are laid out from.
        assert_eq!(
            width(&FONT_MID, "-"),
            MID_MINUS_W,
            "FONT_MID minus advance changed; update MID_MINUS_W"
        );
        assert_eq!(
            width(&FONT_BIG, "%"),
            BIG_PCT_W,
            "FONT_BIG percent advance changed; update BIG_PCT_W"
        );
        assert_eq!(
            width(&FONT_SMALL, "W"),
            SMALL_W_W,
            "FONT_SMALL \"W\" advance changed; update SMALL_W_W"
        );
        assert_eq!(
            width(&FONT_SMALL, "°C"),
            SMALL_DEG_C_W,
            "FONT_SMALL \"°C\" advance changed; update SMALL_DEG_C_W"
        );
        assert_eq!(
            width(&FONT_NET, "-"),
            NET_MINUS_W,
            "FONT_NET minus advance changed; update NET_MINUS_W"
        );
        assert_eq!(
            width(&FONT_MID, ":"),
            MID_COLON_W,
            "FONT_MID colon advance changed; update MID_COLON_W"
        );

        // Every value font must be tabular, or right-aligned values shuffle as
        // their digits change.
        for (name, font, h, w) in [
            ("FONT_BIG", &FONT_BIG, BIG_DIGIT_H, BIG_DIGIT_W),
            ("FONT_NET", &FONT_NET, NET_DIGIT_H, NET_DIGIT_W),
            ("FONT_MID", &FONT_MID, MID_DIGIT_H, MID_DIGIT_W),
        ] {
            assert_eq!(digit_height(font), h, "{name} digit height changed");
            assert_eq!(width(font, "0"), w, "{name} digit advance changed");
            for d in ["1", "4", "9"] {
                assert_eq!(
                    width(font, d),
                    w,
                    "{name} digit {d:?} is not the same width as '0'"
                );
            }
        }
    }
}
