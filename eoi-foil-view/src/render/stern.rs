//! Stern view: the hull at its roll with a dashed ghost at the demanded roll, and the
//! front wings by span (their AoA is not visible end-on, so it is printed).
//!
//! The scene has its own band between the readouts and the sensor row, and the drawn
//! keel lift is clamped to it (the side view shows the true height), so the hull
//! never covers text.

use tiny_skia::Transform;

use super::text::{Align, Face, fmt_height_cm, fmt_signed};
use super::*;

pub fn draw(c: &mut Canvas, r: Bx, cfg: &BoatConfig, v: &View) {
    c.panel(r, "STERN VIEW");
    let s = c.s;
    let (hull, height) = (&cfg.hull, &cfg.height);
    // As in the side view: the roll alone poses the hull, at rest height if need be.
    let known = v.roll_deg.is_some();

    let (half, span) = (hull.half_track_m, hull.foil_span_m);
    let (beam, d) = (hull.beam_m, hull.depth_m);
    let strut = height.strut_below_keel_m();
    let k = (r.w - 60.0 * s) / (2.0 * half + span + 0.5);
    let water_y = r.y + 140.0 * s;
    let floor = r.bottom() - 56.0 * s;
    let band = Bx::new(
        r.x + 10.0 * s,
        r.y + 62.0 * s,
        r.w - 20.0 * s,
        r.h - 120.0 * s,
    );
    let id = Transform::identity();

    c.fill(
        rect(Bx::new(
            r.x + 10.0 * s,
            water_y,
            r.w - 20.0 * s,
            floor - water_y,
        )),
        C_WATER,
        id,
    );
    c.stroke(
        line(r.x + 10.0 * s, water_y, r.right() - 10.0 * s, water_y),
        pen(C_WATERLINE, 2.0 * s),
        id,
    );

    let b = beam / 2.0 * k;
    let section = [
        (-b, -d * k),
        (b, -d * k),
        (b * 0.82, -d * 0.25 * k),
        (0.0, 0.0),
        (-b * 0.82, -d * 0.25 * k),
    ];
    // the struts leave the chine
    let y_top = -d * 0.25 * k * (half / (beam / 2.0)).min(1.0);
    let struts = [-half * k, half * k];
    let bar_h = 3.0 * s;
    let bars: Vec<(f32, f32)> = struts
        .iter()
        .flat_map(|&x| {
            [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)]
                .map(|(dx, dy)| (x + dx * span / 2.0 * k, strut * k + dy * bar_h))
        })
        .collect();

    let roll = v.roll_deg.unwrap_or(0.0);
    let keel_z = v.height_m.unwrap_or(height.rest_m) - height.rest_m;
    let up = section
        .iter()
        .map(|&(x, y)| -rotate(x, y, roll).1)
        .fold(f32::MIN, f32::max);
    let down = bars
        .iter()
        .map(|&(x, y)| rotate(x, y, roll).1)
        .fold(f32::MIN, f32::max);
    let (lo, hi) = (band.y + up, band.bottom() - down);
    let oy = water_y - keel_z * k;
    let oy = if lo <= hi {
        oy.clamp(lo, hi)
    } else {
        (lo + hi) / 2.0
    };
    let origin = Transform::from_translate(r.center_x(), oy);

    c.set_clip(Some(band));
    if let Some(droll) = v.des_roll_deg {
        let t = origin.pre_rotate(droll);
        c.stroke(
            polygon(&section),
            pen(C_ATTITUDE.faded(150), 1.5 * s).dashed(),
            t,
        );
    }

    let (outline, fill, strut_pen) = hull_pens(known, s);
    // starboard down = clockwise
    let t = origin.pre_rotate(roll);
    c.fill(polygon(&section), fill, t);
    c.stroke(polygon(&section), outline, t);
    for &x in &struts {
        c.stroke(line(x, y_top, x, strut * k), strut_pen, t);
    }
    if known {
        // no foils on a guessed pose
        let wings = [
            (struts[0], v.fl_aoa_deg, v.fl_stop, C_FL),
            (struts[1], v.fr_aoa_deg, v.fr_stop, C_FR),
        ];
        for (x, aoa, stop, color) in wings {
            let color = match (stop, aoa) {
                (true, _) => C_ALERT,
                (false, Some(_)) => color,
                (false, None) => color.faded(90),
            };
            let bar = Bx::new(x - span / 2.0 * k, strut * k - bar_h, span * k, 2.0 * bar_h);
            c.fill(rounded_rect(bar, bar_h), color, t);
        }
    }
    c.set_clip(None);

    // Readouts, outside the scene band.
    let right = r.right() - 18.0 * s;
    let roll_s = format!("roll {}", fmt_signed(v.roll_deg, 1, "°"));
    c.text_aligned(
        right,
        r.y + 14.0 * s,
        &roll_s,
        24.0,
        Face::Head,
        C_ATTITUDE,
        Align::Right,
    );
    let demand = format!("demand {}", fmt_signed(v.des_roll_deg, 1, "°"));
    let dim = C_ATTITUDE.faded(180);
    c.text_aligned(
        right,
        r.y + 46.0 * s,
        &demand,
        14.0,
        Face::Bold,
        dim,
        Align::Right,
    );

    let mut x = r.x + 18.0 * s;
    for (name, aoa, stop, color) in [
        ("FL", v.fl_aoa_deg, v.fl_stop, C_FL),
        ("FR", v.fr_aoa_deg, v.fr_stop, C_FR),
    ] {
        let color = match (stop, aoa) {
            (true, _) => C_ALERT,
            (false, Some(_)) => color,
            (false, None) => color.faded(150),
        };
        let wl = c.text(x, r.y + 42.0 * s, name, 12.0, Face::Bold, color);
        c.text(
            x + wl + 6.0 * s,
            r.y + 40.0 * s,
            &fmt_signed(aoa, 1, "°"),
            15.0,
            Face::Head,
            color,
        );
        x += 80.0 * s;
    }

    // The raw strut height sensors.
    let y = r.bottom() - 40.0 * s;
    let half_w = (r.w - 36.0 * s) / 2.0;
    for (j, (name, val)) in [("FL", v.fl_sensor_m), ("FR", v.fr_sensor_m)]
        .into_iter()
        .enumerate()
    {
        let cx = r.x + 18.0 * s + j as f32 * half_w;
        c.text(
            cx,
            y - 14.0 * s,
            &format!("{name} SENSOR"),
            11.0,
            Face::Bold,
            C_DIM,
        );
        if val.is_some() {
            c.text(
                cx,
                y + 2.0 * s,
                &fmt_height_cm(val),
                22.0,
                Face::Head,
                C_TEXT,
            );
        } else {
            c.text(
                cx,
                y + 6.0 * s,
                "no data",
                15.0,
                Face::Bold,
                C_ALERT.faded(230),
            );
        }
    }
}
