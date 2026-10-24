//! Whole frames, checked by what is drawn where rather than by hash: anti-aliasing
//! may differ between x86 and the Pi, but a foil is either on screen or it is not.
//!
//! `SCREENSHOT_DIR=/tmp/shots cargo test -p eoi-foil-view` also saves each frame as a
//! PNG, for looking at.

use eoi_foil_view::render::{self, Bx, C_ALERT, C_FL, C_FR, Fonts, Layout, Rgba};
use eoi_foil_view::source::{RawFrame, apply, run_demo_until};
use eoi_foil_view::state::Model;
use tiny_skia::Pixmap;

fn render(name: &str, model: &mut Model, now: f64, size: (u32, u32)) -> Pixmap {
    let fonts = Fonts::new();
    let view = model.frame(now);
    let mut pm = Pixmap::new(size.0, size.1).unwrap();
    render::draw(&mut pm, &fonts, &model.cfg, &view, &model.history, now);
    if let Ok(dir) = std::env::var("SCREENSHOT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        pm.save_png(format!("{dir}/{name}.png")).unwrap();
    }
    pm
}

/// Pixels in `b` within a small distance of `color` (exact for solid fills, and
/// tolerant of anti-aliased edges).
fn count(pm: &Pixmap, b: Bx, color: Rgba) -> usize {
    let (x0, y0) = (b.x.max(0.0) as u32, b.y.max(0.0) as u32);
    let (x1, y1) = (
        (b.right() as u32).min(pm.width()),
        (b.bottom() as u32).min(pm.height()),
    );
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pm.pixel(x, y).unwrap();
            let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs();
            if d(p.red(), color.0) + d(p.green(), color.1) + d(p.blue(), color.2) < 24 {
                n += 1;
            }
        }
    }
    n
}

/// The scene band of the stern view, where the wings are drawn (no text).
fn stern_scene(l: &Layout) -> Bx {
    let (r, s) = (l.stern, l.s);
    Bx::new(
        r.x + 10.0 * s,
        r.y + 62.0 * s,
        r.w - 20.0 * s,
        r.h - 120.0 * s,
    )
}

#[test]
fn nothing_received_draws_no_foils() {
    let mut model = Model::default();
    let pm = render("stale", &mut model, 5.0, (800, 480));
    let l = Layout::new(800, 480);
    assert_eq!(count(&pm, stern_scene(&l), C_FL), 0);
    assert_eq!(count(&pm, stern_scene(&l), C_FR), 0);
}

#[test]
fn foiling_draws_both_front_wings() {
    let mut model = Model::default();
    run_demo_until(&mut model, 0.0, 20.0);
    let pm = render("demo-20s", &mut model, 20.0, (800, 480));
    let l = Layout::new(800, 480);
    assert!(count(&pm, stern_scene(&l), C_FL) > 50);
    assert!(count(&pm, stern_scene(&l), C_FR) > 50);
    assert_eq!(count(&pm, stern_scene(&l), C_ALERT), 0);
}

#[test]
fn a_wing_at_its_stop_turns_red() {
    let mut model = Model::default();
    run_demo_until(&mut model, 0.0, 31.0);
    assert!(
        model.frame(31.0).fl_stop,
        "the demo puts FL at its stop at 31 s"
    );
    let pm = render("demo-31s-stop", &mut model, 31.0, (800, 480));
    let l = Layout::new(800, 480);
    assert!(count(&pm, stern_scene(&l), C_ALERT) > 50);
    assert_eq!(count(&pm, stern_scene(&l), C_FL), 0);
}

#[test]
fn manual_is_shaded_on_the_chart() {
    let mut model = Model::default();
    run_demo_until(&mut model, 0.0, 9.0);
    let pm = render("demo-9s-takeoff", &mut model, 9.0, (800, 480));
    // The demo switches to FBWA at 4 s: the left half of the last 10 s is MANUAL, so
    // the plot's left edge is lighter than its right.
    let l = Layout::new(800, 480);
    let (plot_x, plot_w) = (l.strip.x + 140.0 * l.s, l.strip.w - 160.0 * l.s);
    let x_ago = |age: f32| (plot_x + plot_w - age / 10.0 * plot_w) as u32;
    let plot_y = (l.strip.y + 50.0 * l.s) as u32;
    let manual = pm.pixel(x_ago(7.0), plot_y).unwrap();
    let fbwa = pm.pixel(x_ago(1.0), plot_y).unwrap();
    assert!(manual.green() > fbwa.green(), "{manual:?} vs {fbwa:?}");
}

#[test]
fn other_screen_sizes_lay_out() {
    for size in [(1024, 600), (1280, 720), (480, 320)] {
        let mut model = Model::default();
        run_demo_until(&mut model, 0.0, 20.0);
        let name = format!("demo-{}x{}", size.0, size.1);
        let pm = render(&name, &mut model, 20.0, size);
        let l = Layout::new(size.0, size.1);
        assert!(count(&pm, stern_scene(&l), C_FL) > 10, "{name}");
    }
}

#[test]
fn on_the_bench_attitude_without_height_still_poses_the_hull() {
    // The autopilot on the bench: attitude and wing PWMs, but the EKF has no height.
    let mut model = Model::default();
    for k in 0..10 {
        let t = f64::from(k) * 0.1;
        // roll +5.00, pitch -2.00
        apply(
            &mut model,
            &RawFrame::standard(0x250, &[0xF4, 0x01, 0x38, 0xFF, 0, 0]),
            t,
        );
        apply(
            &mut model,
            &RawFrame::standard(0x251, &[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]),
            t,
        );
        apply(
            &mut model,
            &RawFrame::standard(0x252, &[0xDC, 0x05, 0x8C, 0x05]),
            t,
        );
    }
    let view = model.frame(0.9);
    assert_eq!(view.height_m, None);
    let pm = render("bench-no-height", &mut model, 0.9, (800, 480));
    let l = Layout::new(800, 480);
    assert!(
        count(&pm, stern_scene(&l), C_FL) > 50,
        "wings drawn on a posed hull"
    );
}
