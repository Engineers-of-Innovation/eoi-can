//! Side view: the hull at its pitch and ride height above the water, the struts, and
//! the foils at their AoA (the rear with a dashed ghost at its command).

use tiny_skia::Transform;

use super::text::{Align, Face, fmt_height_cm, fmt_signed};
use super::*;

/// One foil at `aoa` (degrees, nose up = counter-clockwise), `chord` px long, pivoting
/// about its quarter chord at `at` in the frame `base`.
#[allow(clippy::too_many_arguments)]
pub fn foil(
    c: &mut Canvas,
    base: Transform,
    at: (f32, f32),
    chord: f32,
    aoa: Option<f32>,
    color: Rgba,
    stop: bool,
    ghost: bool,
    alpha: u8,
) {
    let Some(aoa) = aoa else { return };
    let t = base
        .pre_translate(at.0, at.1)
        .pre_rotate(-aoa)
        .pre_scale(chord, chord);
    let color = if stop { C_ALERT } else { color };
    if ghost {
        c.stroke(
            foil_path(),
            pen(color.faded(210), 1.6 * c.s / chord).dashed(),
            t,
        );
    } else {
        c.fill(foil_path(), color.faded(alpha), t);
    }
}

pub fn draw(c: &mut Canvas, r: Bx, cfg: &BoatConfig, v: &View) {
    c.panel(r, "SIDE VIEW");
    let s = c.s;
    let (hull, height) = (&cfg.hull, &cfg.height);
    // The pose follows the pitch whenever there is one. Without a ride height (on the
    // bench, or the EKF has none) the hull sits at its rest height; only without a
    // pitch is it a dashed guess.
    let known = v.pitch_deg.is_some();

    // The scene, in true proportions; the foils are drawn at 8x chord so their angle reads.
    let k = (r.w - 70.0 * s) / hull.length_m;
    let x_ff = hull.front_foil_x_m;
    let water_y = r.y + 172.0 * s;
    let floor = r.bottom() - 56.0 * s;
    let sx = |xm: f32| r.x + 35.0 * s + xm * k;
    let id = Transform::identity();

    c.set_clip(Some(Bx::new(
        r.x + 10.0 * s,
        r.y + 10.0 * s,
        r.w - 20.0 * s,
        floor - r.y - 10.0 * s,
    )));
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
    let wave: Vec<(f32, f32)> = (0..64)
        .map(|j| {
            let x = r.x + 10.0 * s + j as f32 * (r.w - 20.0 * s) / 63.0;
            (x, water_y + 1.6 * s * (j as f32 * 0.85).sin())
        })
        .collect();
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(wave[0].0, wave[0].1);
    for &(x, y) in &wave[1..] {
        pb.line_to(x, y);
    }
    c.stroke(pb.finish(), pen(C_WATERLINE, 2.0 * s), id);

    // The hull frame: origin on the keel at the front strut, bow up = counter-clockwise.
    let keel_z = v.height_m.unwrap_or(height.rest_m) - height.rest_m;
    let strut = height.strut_below_keel_m();
    let pitch = v.pitch_deg.unwrap_or(0.0);
    let t = Transform::from_translate(sx(x_ff), water_y - keel_z * k).pre_rotate(-pitch);
    let bx = |xm: f32| (xm - x_ff) * k;
    let z = |zm: f32| -zm * k;

    let (outline, fill, strut_pen) = hull_pens(known, s);
    let (d, l) = (hull.depth_m, hull.length_m);
    let shape = polygon(&[
        (bx(0.0), z(0.06)),
        (bx(0.0), z(d)),
        (bx(l), z(d + 0.05)),
        (bx(l - 0.35), z(d * 0.55)),
        (bx(l - 1.1), z(0.04)),
        (bx(l - 1.9), z(0.0)),
        (bx(0.35), z(0.0)),
    ]);
    c.fill(shape.clone(), fill, t);
    c.stroke(shape, outline, t);
    if known {
        // the solar deck
        c.stroke(
            line(bx(0.6), z(d + 0.03), bx(l - 1.4), z(d + 0.035)),
            pen(TIDAL_TURQUOISE.faded(120), 3.0 * s),
            t,
        );
    }
    for (xm, keel) in [(x_ff, 0.0), (hull.rear_foil_x_m, 0.05)] {
        c.stroke(line(bx(xm), z(keel), bx(xm), z(-strut)), strut_pen, t);
    }

    // The water crosses the strut at H - rest below the keel: mark breach and target.
    let bz = -(height.breach_m - height.rest_m);
    let alert = if known { C_ALERT } else { C_ALERT.faded(110) };
    c.stroke(
        line(bx(x_ff) - 9.0 * s, z(bz), bx(x_ff) + 9.0 * s, z(bz)),
        pen(alert, 3.0 * s),
        t,
    );
    if let Some(tgt) = v.target_m {
        let color = if known { C_TARGET } else { C_TARGET.faded(110) };
        let tz = -(tgt - height.rest_m);
        c.stroke(
            line(bx(x_ff) - 13.0 * s, z(tz), bx(x_ff) + 13.0 * s, z(tz)),
            pen(color, 2.0 * s).dashed(),
            t,
        );
    }

    if known {
        // no foils on a guessed pose
        let chord = hull.foil_chord_m * k * 8.0;
        let front = (bx(x_ff), z(-strut));
        foil(
            c,
            t,
            front,
            chord,
            v.fl_aoa_deg,
            C_FL,
            v.fl_stop,
            false,
            255,
        );
        foil(
            c,
            t,
            front,
            chord,
            v.fr_aoa_deg,
            C_FR,
            v.fr_stop,
            false,
            170,
        );
        let rear = (bx(hull.rear_foil_x_m), z(-strut));
        foil(
            c,
            t,
            rear,
            chord,
            v.rear_cmd_aoa_deg,
            C_REAR,
            false,
            true,
            255,
        );
        foil(c, t, rear, chord, v.rear_aoa_deg, C_REAR, false, false, 255);
    }
    c.set_clip(None);

    // Readouts, over the scene.
    let right = r.right() - 18.0 * s;
    let pitch_s = format!("pitch {}", fmt_signed(v.pitch_deg, 1, "°"));
    c.text_aligned(
        right,
        r.y + 14.0 * s,
        &pitch_s,
        24.0,
        Face::Head,
        C_ATTITUDE,
        Align::Right,
    );
    let demand = format!("demand {}", fmt_signed(v.des_pitch_deg, 1, "°"));
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
    let left = r.x + 18.0 * s;
    let wh = c.text(
        left,
        r.y + 40.0 * s,
        &fmt_height_cm(v.height_m),
        30.0,
        Face::Head,
        C_HEIGHT,
    );
    c.text(
        left + wh + 10.0 * s,
        r.y + 50.0 * s,
        "ride height",
        13.0,
        Face::Bold,
        C_HEIGHT,
    );
    if v.target_m.is_some() {
        let target = format!("target {}", fmt_height_cm(v.target_m));
        c.text(left, r.y + 80.0 * s, &target, 14.0, Face::Bold, C_TARGET);
    }

    let y = r.bottom() - 40.0 * s;
    let cells = [
        ("FL", v.fl_aoa_deg, C_FL, v.fl_stop),
        ("FR", v.fr_aoa_deg, C_FR, v.fr_stop),
        ("REAR", v.rear_aoa_deg, C_REAR, false),
    ];
    let cw = (r.w - 36.0 * s) / 3.0;
    for (j, (name, val, color, stop)) in cells.into_iter().enumerate() {
        let cx = left + j as f32 * cw;
        let color = if stop { C_ALERT } else { color };
        let wl = c.text(cx, y + 6.0 * s, name, 13.0, Face::Bold, color);
        c.text(
            cx + wl + 8.0 * s,
            y,
            &fmt_signed(val, 1, "°"),
            24.0,
            Face::Head,
            color,
        );
        if stop {
            c.text(
                cx + wl + 8.0 * s,
                y - 18.0 * s,
                "AT STOP",
                11.0,
                Face::Bold,
                C_ALERT,
            );
        }
    }
}
