//! The foiling screen: trim and tuning parameters for the foil system.
//!
//! Geometry only -- the `Cell` model, the fonts and every widget come from
//! [`super`], exactly as [`super::dashboard`] uses them. What lives here is
//! where things go, and the compile-time assertions that keep them from
//! colliding as the constants are retuned.
//!
//! Placeholder: the layout is not designed yet, and none of the parameters it
//! is meant to show are decoded from the bus. It draws the stamp band and a
//! title so the plumbing -- firmware bin, simulator `--layout`, framebuffer --
//! can be built and seen end to end before the geometry exists.

use core::fmt::Write;

use embedded_graphics::{pixelcolor::BinaryColor, prelude::*, text::Alignment};
use heapless::String;
use u8g2_fonts::types::HorizontalAlignment;

use super::*;
use crate::{built_info, DisplayData};

/// Centre of the screen, for the placeholder title. Real geometry replaces this.
const TITLE_CENTER_Y: i32 = SCREEN.h / 2;

// The title has to clear the stamp band above it -- the one rule the real layout
// will also have to keep. Const arithmetic, so it is a compile-time check, as
// every geometry assertion in `dashboard` is.
const _: () = assert!(TITLE_CENTER_Y - SMALL_CAP_H / 2 >= STAMP_BAND_H);

/// Only `FONT_SMALL` carries letters: the three value fonts are tabular-numeral
/// subsets (" -.:0123456789"), so any text on this screen is 14px until someone
/// builds a larger `_tf` face with `support/build-fonts.py`. Rendering a letter
/// in `FONT_NET`/`FONT_BIG`/`FONT_MID` panics in `map_font_err`.
const FONT_TITLE: &FontRenderer = &FONT_SMALL;

pub fn draw_foiling<D, C>(display: &mut D, data: &DisplayData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = C>,
    C: PixelColor + From<BinaryColor>,
{
    display.clear(BinaryColor::On.into())?;

    draw_text(
        display,
        FONT_TITLE,
        HorizontalAlignment::Center,
        SCREEN.center_x(),
        TITLE_CENTER_Y,
        "Foiling trim and tuning",
    )?;

    draw_version(display)?;
    draw_ip(display, data)
}

/// The build stamp, centred as the dashboard's is. Kept per-layout rather than
/// shared: which x a stamp is anchored to is a layout decision.
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
        SCREEN.center_x(),
        Alignment::Center,
        version.as_str(),
    )
}

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

    draw_stamp(display, 0, Alignment::Left, ip.as_str())
}
