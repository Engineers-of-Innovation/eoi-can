//! Text as filled glyph outlines, so it is anti-aliased and blended like every other
//! shape.
//!
//! The font is Raleway (OFL, `assets/OFL.txt`), the brand book's text face, used
//! unmodified: a variable font set to three weights, with its lining figures
//! (`zero.lf`...) picked by glyph name, because the default figures are old-style and
//! there is no shaper here to apply `lnum`. Digits, signs and the figure space sit in
//! fixed-width cells, so a changing number does not jitter.

use std::cell::RefCell;
use std::collections::HashMap;

use ab_glyph::{Font, FontRef, GlyphId, OutlineCurve, VariableFont};
use tiny_skia::{FillRule, Path, PathBuilder, Pixmap, Transform};

use super::Rgba;

const RALEWAY: &[u8] = include_bytes!("../../assets/Raleway.ttf");

const DIGIT_NAMES: [&str; 10] = [
    "zero.lf", "one.lf", "two.lf", "three.lf", "four.lf", "five.lf", "six.lf", "seven.lf",
    "eight.lf", "nine.lf",
];
pub const MINUS: char = '\u{2212}';
pub const FIGURE_SPACE: char = '\u{2007}';
pub const DASH: &str = "\u{2014}";

/// Which of the three weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    /// Big numbers and readouts (the overlay's Clash Display Medium).
    Head,
    /// Ordinary labels, SemiBold.
    Label,
    /// Panel titles and small caps-like labels.
    Bold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

struct Weight {
    font: FontRef<'static>,
    /// Unscaled widths of the tabular cells: one for digits, one for signs and the
    /// figure space.
    digit: f32,
    sign: f32,
}

pub struct Fonts {
    head: Weight,
    label: Weight,
    bold: Weight,
    lnum: [GlyphId; 10],
    upem: f32,
    cap_height: f32,
    paths: RefCell<HashMap<(Face, u16), Option<Path>>>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        let face = ttf_parser::Face::parse(RALEWAY, 0).expect("Raleway parses");
        let lnum = DIGIT_NAMES.map(|name| {
            GlyphId(
                face.glyph_index_by_name(name)
                    .expect("Raleway has lining figures")
                    .0,
            )
        });
        let upem = f32::from(face.units_per_em());
        let cap_height = f32::from(face.capital_height().unwrap_or(700));
        let weight = |wght: f32| {
            let mut font = FontRef::try_from_slice(RALEWAY).expect("Raleway loads");
            font.set_variation(b"wght", wght);
            let widest = |glyphs: &[GlyphId]| {
                glyphs
                    .iter()
                    .map(|&g| font.h_advance_unscaled(g))
                    .fold(0.0, f32::max)
            };
            let digit = widest(&lnum);
            let sign = widest(&['+', MINUS].map(|c| font.glyph_id(c)));
            Weight { font, digit, sign }
        };
        Self {
            head: weight(500.0),
            label: weight(600.0),
            bold: weight(700.0),
            lnum,
            upem,
            cap_height,
            paths: RefCell::new(HashMap::new()),
        }
    }

    fn weight(&self, face: Face) -> &Weight {
        match face {
            Face::Head => &self.head,
            Face::Label => &self.label,
            Face::Bold => &self.bold,
        }
    }

    /// (glyph, x offset) in font units, and the total advance.
    fn layout(&self, face: Face, text: &str) -> (Vec<(GlyphId, f32)>, f32) {
        let w = self.weight(face);
        let mut x = 0.0;
        let mut out = Vec::with_capacity(text.len());
        for c in text.chars() {
            let (glyph, cell) = match c {
                '0'..='9' => (Some(self.lnum[c as usize - '0' as usize]), w.digit),
                '+' | MINUS => (Some(w.font.glyph_id(c)), w.sign),
                FIGURE_SPACE => (None, w.sign),
                _ => {
                    let g = w.font.glyph_id(c);
                    out.push((g, x));
                    x += w.font.h_advance_unscaled(g);
                    continue;
                }
            };
            if let Some(g) = glyph {
                out.push((g, x + (cell - w.font.h_advance_unscaled(g)) / 2.0));
            }
            x += cell;
        }
        (out, x)
    }

    pub fn width(&self, face: Face, text: &str, px: f32) -> f32 {
        self.layout(face, text).1 * px / self.upem
    }

    fn glyph_path(&self, face: Face, glyph: GlyphId) -> Option<Path> {
        self.paths
            .borrow_mut()
            .entry((face, glyph.0))
            .or_insert_with(|| {
                let outline = self.weight(face).font.outline(glyph)?;
                let mut pb = PathBuilder::new();
                let mut pen = None;
                for curve in &outline.curves {
                    let (start, end) = match *curve {
                        OutlineCurve::Line(a, b) => (a, b),
                        OutlineCurve::Quad(a, _, b) => (a, b),
                        OutlineCurve::Cubic(a, _, _, b) => (a, b),
                    };
                    if pen != Some(start) {
                        if pen.is_some() {
                            pb.close();
                        }
                        pb.move_to(start.x, start.y);
                    }
                    match *curve {
                        OutlineCurve::Line(_, b) => pb.line_to(b.x, b.y),
                        OutlineCurve::Quad(_, c, b) => pb.quad_to(c.x, c.y, b.x, b.y),
                        OutlineCurve::Cubic(_, c1, c2, b) => {
                            pb.cubic_to(c1.x, c1.y, c2.x, c2.y, b.x, b.y)
                        }
                    }
                    pen = Some(end);
                }
                pb.close();
                pb.finish()
            })
            .clone()
    }

    /// Draws `text` with the top of its capitals at `y`, and returns its width.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        pm: &mut Pixmap,
        x: f32,
        y: f32,
        text: &str,
        px: f32,
        face: Face,
        color: Rgba,
        align: Align,
    ) -> f32 {
        let (glyphs, advance) = self.layout(face, text);
        let k = px / self.upem;
        let width = advance * k;
        let left = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        let baseline = y + self.cap_height * k;
        let paint = color.paint();
        for (glyph, gx) in glyphs {
            if let Some(path) = self.glyph_path(face, glyph) {
                let t = Transform::from_translate(left + gx * k, baseline).pre_scale(k, -k);
                pm.fill_path(&path, &paint, FillRule::Winding, t, None);
            }
        }
        width
    }
}

/// Signed with fixed decimals and a true minus; a value that rounds to zero gets a
/// figure space instead of a sign. "—" when missing.
pub fn fmt_signed(v: Option<f32>, digits: usize, unit: &str) -> String {
    let Some(v) = v.filter(|v| v.is_finite()) else {
        return DASH.into();
    };
    let s = format!("{v:+.digits$}");
    let s = if s[1..].parse::<f32>() == Ok(0.0) {
        format!("{FIGURE_SPACE}{}", &s[1..])
    } else {
        s.replace('-', &MINUS.to_string())
    };
    s + unit
}

pub fn fmt_height_cm(v: Option<f32>) -> String {
    match v.filter(|v| v.is_finite()) {
        Some(m) => {
            let s = format!("{:.0}", m * 100.0);
            let s = if s == "-0" {
                "0".into()
            } else {
                s.replace('-', &MINUS.to_string())
            };
            s + " cm"
        }
        None => DASH.into(),
    }
}

/// `format(v, "g")` for the round numbers on the chart's range labels.
pub fn fmt_g(v: f32) -> String {
    let s = if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    };
    s.replace('-', &MINUS.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_formatting_matches_the_overlay() {
        assert_eq!(fmt_signed(Some(2.44), 1, "°"), "+2.4°");
        assert_eq!(fmt_signed(Some(-3.06), 1, "°"), "\u{2212}3.1°");
        assert_eq!(fmt_signed(Some(-0.04), 1, "°"), "\u{2007}0.0°");
        assert_eq!(fmt_signed(None, 1, "°"), "\u{2014}");
        assert_eq!(fmt_height_cm(Some(0.314)), "31 cm");
        assert_eq!(fmt_height_cm(Some(-0.004)), "0 cm");
        assert_eq!(
            (fmt_g(0.6), fmt_g(-8.0)),
            ("0.6".into(), "\u{2212}8".into())
        );
    }

    #[test]
    fn numbers_are_tabular() {
        let fonts = Fonts::new();
        let w = |s| fonts.width(Face::Head, s, 24.0);
        assert_eq!(w("+1.1°"), w("+8.8°"));
        assert_eq!(w("\u{2212}0.4"), w("+7.0"));
        assert_eq!(w("\u{2007}0.0"), w("+0.1"));
    }

    #[test]
    fn every_character_we_draw_is_in_the_font() {
        let fonts = Fonts::new();
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz °…—·−+.%".chars() {
            assert_ne!(fonts.label.font.glyph_id(c).0, 0, "{c:?}");
        }
    }
}
