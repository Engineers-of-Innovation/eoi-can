//! The strip chart: the trailing 10 s in three lanes (height, pitch, wing AoA),
//! shaded where the flight controller was in MANUAL.

use tiny_skia::{PathBuilder, Transform};

use super::text::{Align, Face, MINUS, fmt_g};
use super::*;
use crate::state::MODE_MANUAL;

type Series = (fn(&View) -> Option<f32>, Pen);

struct Lane {
    name: &'static str,
    color: Rgba,
    min: f32,
    max: f32,
    unit: &'static str,
    series: Vec<Series>,
}

fn lanes(s: f32) -> [Lane; 3] {
    [
        Lane {
            name: "HEIGHT",
            color: C_HEIGHT,
            min: 0.0,
            max: 0.6,
            unit: "m",
            series: vec![
                (|v| v.height_m, pen(C_HEIGHT, 2.6 * s)),
                (|v| v.target_m, pen(C_TARGET, 1.5 * s).dashed()),
            ],
        },
        Lane {
            name: "PITCH",
            color: C_ATTITUDE,
            min: -8.0,
            max: 8.0,
            unit: "°",
            series: vec![
                (|v| v.pitch_deg, pen(C_ATTITUDE, 2.4 * s)),
                (
                    |v| v.des_pitch_deg,
                    pen(C_ATTITUDE.faded(150), 1.5 * s).dashed(),
                ),
            ],
        },
        Lane {
            name: "WING AoA",
            color: C_TEXT,
            min: -8.0,
            max: 10.0,
            unit: "°",
            series: vec![
                (|v| v.fl_aoa_deg, pen(C_FL, 2.2 * s)),
                (|v| v.fr_aoa_deg, pen(C_FR, 2.2 * s)),
                (|v| v.rear_aoa_deg, pen(C_REAR, 2.2 * s)),
            ],
        },
    ]
}

/// A line through the points, broken wherever a value is missing.
fn polyline(points: impl Iterator<Item = (f32, Option<f32>)>) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut down = false;
    for (x, y) in points {
        match y {
            None => down = false,
            Some(y) if down => pb.line_to(x, y),
            Some(y) => {
                pb.move_to(x, y);
                down = true;
            }
        }
    }
    pb.finish()
}

pub fn draw(c: &mut Canvas, r: Bx, cfg: &BoatConfig, history: &History, now: f64) {
    let span = History::SPAN_S as f32;
    c.panel(r, &format!("LAST {span:.0} S"));
    let s = c.s;
    let id = Transform::identity();
    let plot = Bx::new(
        r.x + 140.0 * s,
        r.y + 40.0 * s,
        r.w - 160.0 * s,
        r.h - 70.0 * s,
    );
    let x_of = |age: f64| plot.right() - age as f32 / span * plot.w;
    let samples: Vec<(f32, &View)> = history
        .iter(now)
        .filter(|(age, _)| *age <= History::SPAN_S)
        .map(|(age, v)| (x_of(age), v))
        .collect();

    // MANUAL shading, run by run.
    let mut run: Option<f32> = None;
    for (j, &(x, v)) in samples.iter().enumerate() {
        let manual = v.mode == Some(MODE_MANUAL);
        if manual && run.is_none() {
            run = Some(x);
        }
        let last = j + 1 == samples.len();
        if let Some(x0) = run
            && (!manual || last)
        {
            let x1 = if manual { x } else { samples[j - 1].0 };
            c.fill(
                rect(Bx::new(x0, plot.y, x1 - x0 + 1.0, plot.h)),
                C_MANUAL,
                id,
            );
            run = None;
        }
    }

    let lanes = lanes(s);
    let lane_h = plot.h / lanes.len() as f32;
    for (j, lane) in lanes.iter().enumerate() {
        let top = plot.y + j as f32 * lane_h;
        let area = Bx::new(plot.x, top + 4.0 * s, plot.w, lane_h - 8.0 * s);
        let to_y = |v: f32| {
            area.bottom()
                - (v.clamp(lane.min, lane.max) - lane.min) / (lane.max - lane.min) * area.h
        };

        let mid = top + lane_h / 2.0;
        c.text(
            r.x + 18.0 * s,
            mid - 14.0 * s,
            lane.name,
            13.0,
            Face::Bold,
            lane.color,
        );
        let range = format!("{} … {} {}", fmt_g(lane.min), fmt_g(lane.max), lane.unit);
        c.text(
            r.x + 18.0 * s,
            mid + 4.0 * s,
            &range,
            11.0,
            Face::Bold,
            C_DIM,
        );
        c.stroke(
            line(area.x, area.bottom(), area.right(), area.bottom()),
            pen(C_FAINT, 1.0),
            id,
        );
        if lane.min < 0.0 && 0.0 < lane.max {
            let y0 = to_y(0.0);
            c.stroke(
                line(area.x, y0, area.right(), y0),
                pen(C_FAINT, 1.0).dotted(),
                id,
            );
        }
        if j == 0 {
            let yb = to_y(cfg.height.breach_m);
            c.stroke(
                line(area.x, yb, area.right(), yb),
                pen(C_ALERT.faded(210), 1.5 * s),
                id,
            );
            // The raw front-right sensor, as faint dots.
            let mut pb = PathBuilder::new();
            for &(x, v) in &samples {
                if let Some(m) = v.fr_sensor_m {
                    pb.push_circle(x, to_y(m), 1.3 * s);
                }
            }
            c.fill(pb.finish(), C_FR.faded(60), id);
        }
        for (value, pen) in &lane.series {
            let points = samples.iter().map(|&(x, v)| (x, value(v).map(to_y)));
            c.stroke(polyline(points), *pen, id);
        }
    }

    c.stroke(
        line(plot.right(), plot.y, plot.right(), plot.bottom()),
        pen(C_DIM, 1.5 * s),
        id,
    );
    for sec in (0..=span as u32).step_by(2) {
        let x = x_of(f64::from(sec));
        let label = if sec == 0 {
            "now".to_string()
        } else {
            format!("{MINUS}{sec} s")
        };
        c.text_aligned(
            x,
            plot.bottom() + 8.0 * s,
            &label,
            11.0,
            Face::Bold,
            C_DIM,
            Align::Center,
        );
    }
}
