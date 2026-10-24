//! The frame: a side view, a stern view and a 10 s strip chart, ported from the bottom
//! strip of python-data-analysis `overlay/render.py` (c58198e).
//!
//! The overlay lays its panels out on a 1920x1080 video; here they fill the LCD, and
//! every fixed pixel offset and font size from the overlay is multiplied by
//! [`Canvas::s`], the panel height over the overlay's 290 px.

mod side;
mod stern;
mod strip;
pub mod text;

use tiny_skia::{
    FillRule, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pixmap, Stroke, StrokeDash,
    Transform,
};

use crate::config::BoatConfig;
use crate::state::{History, View};
pub use text::Fonts;
use text::{Align, Face};

/// A straight (not premultiplied) sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn hex(v: u32) -> Self {
        Self((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
    }

    pub const fn faded(self, alpha: u8) -> Self {
        Self(self.0, self.1, self.2, alpha)
    }

    pub fn paint(self) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color_rgba8(self.0, self.1, self.2, self.3);
        paint.anti_alias = true;
        paint
    }
}

// The Engineers of Innovation brand palette.
pub const LAKE_LIME: Rgba = Rgba::hex(0x133131);
pub const OFF_WHITE: Rgba = Rgba::hex(0xf9f4eb);
pub const EHB_ORANGE: Rgba = Rgba::hex(0xff7816);
pub const BLEEK_LIME: Rgba = Rgba::hex(0xb9ff25);
pub const TIDAL_TURQUOISE: Rgba = Rgba::hex(0x1dfadf);
pub const LAKE_LIME_100: Rgba = Rgba::hex(0xb5bfbf);
pub const BLEEK_LIME_200: Rgba = Rgba::hex(0xe2ffa3);
pub const ORANGE_300: Rgba = Rgba::hex(0xffa463);

// One colour per quantity, everywhere.
pub const C_FL: Rgba = TIDAL_TURQUOISE;
pub const C_FR: Rgba = EHB_ORANGE;
pub const C_REAR: Rgba = OFF_WHITE;
pub const C_HEIGHT: Rgba = BLEEK_LIME;
pub const C_TARGET: Rgba = BLEEK_LIME_200;
pub const C_ATTITUDE: Rgba = LAKE_LIME_100;
/// Stops and breach: the one non-brand colour.
pub const C_ALERT: Rgba = Rgba::hex(0xff4d4d);
pub const C_TEXT: Rgba = OFF_WHITE;
pub const C_DIM: Rgba = OFF_WHITE.faded(160);
pub const C_FAINT: Rgba = OFF_WHITE.faded(60);
pub const C_WATER: Rgba = TIDAL_TURQUOISE.faded(34);
pub const C_WATERLINE: Rgba = TIDAL_TURQUOISE.faded(150);
pub const C_HULL: Rgba = LAKE_LIME_100;
pub const C_MANUAL: Rgba = OFF_WHITE.faded(26);
/// The overlay's panels are Lake Lime at 55 % over video. On the LCD there is no
/// video, so the screen is a darker Lake Lime and the panels are the brand colour.
pub const BACKGROUND: Rgba = Rgba::hex(0x0a1c1c);
pub const C_PANEL: Rgba = LAKE_LIME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dash {
    Solid,
    Dashed,
    Dotted,
}

/// Qt's pen: width, round caps and joins, and a dash pattern in pen widths.
#[derive(Debug, Clone, Copy)]
pub struct Pen {
    pub color: Rgba,
    pub width: f32,
    pub dash: Dash,
}

pub fn pen(color: Rgba, width: f32) -> Pen {
    Pen {
        color,
        width,
        dash: Dash::Solid,
    }
}

impl Pen {
    pub fn dashed(self) -> Self {
        Self {
            dash: Dash::Dashed,
            ..self
        }
    }

    pub fn dotted(self) -> Self {
        Self {
            dash: Dash::Dotted,
            ..self
        }
    }

    fn stroke(&self) -> Stroke {
        let w = self.width;
        let dash = match self.dash {
            Dash::Solid => None,
            Dash::Dashed => StrokeDash::new(vec![4.0 * w, 2.0 * w], 0.0),
            Dash::Dotted => StrokeDash::new(vec![1.0 * w, 2.0 * w], 0.0),
        };
        Stroke {
            width: w,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            dash,
            ..Stroke::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bx {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bx {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center_x(&self) -> f32 {
        self.x + self.w / 2.0
    }
}

pub fn polygon(points: &[(f32, f32)]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (x, y) = *points.first()?;
    pb.move_to(x, y);
    for &(x, y) in &points[1..] {
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish()
}

pub fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    pb.finish()
}

pub fn rect(b: Bx) -> Option<Path> {
    tiny_skia::Rect::from_xywh(b.x, b.y, b.w, b.h).map(PathBuilder::from_rect)
}

pub fn rounded_rect(b: Bx, r: f32) -> Option<Path> {
    let r = r.min(b.w / 2.0).min(b.h / 2.0).max(0.0);
    let k = r * 0.552_284_8; // circle-to-cubic control distance
    let (l, t, rt, bt) = (b.x, b.y, b.right(), b.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, bt - r);
    pb.cubic_to(rt, bt - r + k, rt - r + k, bt, rt - r, bt);
    pb.line_to(l + r, bt);
    pb.cubic_to(l + r - k, bt, l, bt - r + k, l, bt - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
}

/// A pixmap with the fonts, the layout scale and an optional clip.
pub struct Canvas<'a> {
    pub pm: &'a mut Pixmap,
    pub fonts: &'a Fonts,
    /// Overlay pixels to screen pixels.
    pub s: f32,
    clip: Option<Mask>,
}

impl<'a> Canvas<'a> {
    pub fn new(pm: &'a mut Pixmap, fonts: &'a Fonts, s: f32) -> Self {
        Self {
            pm,
            fonts,
            s,
            clip: None,
        }
    }

    pub fn set_clip(&mut self, b: Option<Bx>) {
        self.clip = b.and_then(|b| {
            let mut mask = Mask::new(self.pm.width(), self.pm.height())?;
            mask.fill_path(&rect(b)?, FillRule::Winding, false, Transform::identity());
            Some(mask)
        });
    }

    pub fn fill(&mut self, path: Option<Path>, color: Rgba, t: Transform) {
        if let Some(path) = path {
            let paint = color.paint();
            self.pm
                .fill_path(&path, &paint, FillRule::Winding, t, self.clip.as_ref());
        }
    }

    pub fn stroke(&mut self, path: Option<Path>, pen: Pen, t: Transform) {
        if let Some(path) = path {
            let paint = pen.color.paint();
            self.pm
                .stroke_path(&path, &paint, &pen.stroke(), t, self.clip.as_ref());
        }
    }

    /// `px` is in overlay pixels and scaled here; `x`, `y` are screen pixels, `y` the
    /// top of the capitals. Returns the width drawn.
    pub fn text(&mut self, x: f32, y: f32, s: &str, px: f32, face: Face, color: Rgba) -> f32 {
        self.text_aligned(x, y, s, px, face, color, Align::Left)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text_aligned(
        &mut self,
        x: f32,
        y: f32,
        s: &str,
        px: f32,
        face: Face,
        color: Rgba,
        align: Align,
    ) -> f32 {
        self.fonts
            .draw(self.pm, x, y, s, px * self.s, face, color, align)
    }

    /// The rounded panel and its title.
    pub fn panel(&mut self, r: Bx, title: &str) {
        let s = self.s;
        self.fill(rounded_rect(r, 18.0 * s), C_PANEL, Transform::identity());
        self.text(
            r.x + 18.0 * s,
            r.y + 16.0 * s,
            title,
            13.0,
            Face::Bold,
            C_DIM,
        );
    }
}

/// Where the three panels go on a screen of this size.
pub struct Layout {
    pub side: Bx,
    pub stern: Bx,
    pub strip: Bx,
    pub s: f32,
}

impl Layout {
    pub fn new(width: u32, height: u32) -> Self {
        let (w, h) = (width as f32, height as f32);
        let margin = (h * 0.016).round().max(4.0);
        let gap = margin;
        let top_h = ((h - 2.0 * margin - gap) * 0.55).round();
        let side_w = ((w - 2.0 * margin - gap) * 0.62).round();
        let side = Bx::new(margin, margin, side_w, top_h);
        let stern = Bx::new(
            side.right() + gap,
            margin,
            w - margin - side.right() - gap,
            top_h,
        );
        let strip_y = margin + top_h + gap;
        let strip = Bx::new(margin, strip_y, w - 2.0 * margin, h - margin - strip_y);
        Self {
            side,
            stern,
            strip,
            s: top_h / 290.0,
        }
    }
}

/// Draws one whole frame.
pub fn draw(
    pm: &mut Pixmap,
    fonts: &Fonts,
    cfg: &BoatConfig,
    view: &View,
    history: &History,
    now: f64,
) {
    let b = BACKGROUND;
    pm.fill(tiny_skia::Color::from_rgba8(b.0, b.1, b.2, b.3));
    let layout = Layout::new(pm.width(), pm.height());
    let mut c = Canvas::new(pm, fonts, layout.s);
    side::draw(&mut c, layout.side, cfg, view);
    stern::draw(&mut c, layout.stern, cfg, view);
    strip::draw(&mut c, layout.strip, cfg, history, now);
}

/// Unit-chord NACA 4412-like profile, leading edge toward +x, pivot at the quarter
/// chord, y up (so drawn with y flipped on screen).
pub fn foil_path() -> Option<Path> {
    let xs: Vec<f32> = (0..18).map(|k| k as f32 / 17.0).collect();
    let thick = |x: f32| {
        0.6 * (0.2969 * x.sqrt() - 0.126 * x - 0.3516 * x.powi(2) + 0.2843 * x.powi(3)
            - 0.1015 * x.powi(4))
    };
    let camber = |x: f32| {
        0.04 * if x < 0.4 {
            (0.8 * x - x * x) / 0.16
        } else {
            (0.2 + 0.8 * x - x * x) / 0.36
        }
    };
    let upper = xs.iter().map(|&x| (x, camber(x) + thick(x)));
    let lower = xs.iter().rev().map(|&x| (x, camber(x) - thick(x)));
    let pts: Vec<(f32, f32)> = upper.chain(lower).map(|(x, y)| (0.25 - x, -y)).collect();
    polygon(&pts)
}

/// `(x, y)` rotated as a clockwise-on-screen rotation by `deg` would move it.
pub fn rotate(x: f32, y: f32, deg: f32) -> (f32, f32) {
    let (sin, cos) = deg.to_radians().sin_cos();
    (x * cos - y * sin, x * sin + y * cos)
}

/// (outline pen, fill, strut pen): solid for a live pose, a faint dashed ghost when
/// pitch / roll / height is missing and the pose is a default.
pub fn hull_pens(known: bool, s: f32) -> (Pen, Rgba, Pen) {
    if known {
        (
            pen(C_HULL, 2.0 * s),
            LAKE_LIME.faded(200),
            pen(C_HULL, 3.0 * s),
        )
    } else {
        let ghost = C_HULL.faded(110);
        (
            pen(ghost, 1.6 * s).dashed(),
            LAKE_LIME.faded(70),
            pen(ghost, 2.0 * s).dashed(),
        )
    }
}
