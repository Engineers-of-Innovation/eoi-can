use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use tiny_skia::Pixmap;
use tracing_subscriber::EnvFilter;

use eoi_foil_view::config::BoatConfig;
use eoi_foil_view::render::{self, Fonts};
use eoi_foil_view::source::{self, Shared};
use eoi_foil_view::state::Model;

#[cfg(feature = "window")]
mod window;

/// Live attitude, ride height and foil angles on the datalogger's LCD.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// CAN interface to read (the default source).
    #[arg(short, long, default_value = "can0")]
    can_interface: String,

    /// Play a `candump -l` log instead of reading CAN.
    #[arg(long, conflicts_with = "demo")]
    replay: Option<PathBuf>,

    /// Seconds into the replay to start from.
    #[arg(long, default_value_t = 0.0, requires = "replay")]
    from: f64,

    /// Replay speed, 1 = real time.
    #[arg(long, default_value_t = 1.0, requires = "replay")]
    speed: f64,

    /// Show a built-in 60 s demo run instead of reading CAN.
    #[arg(long)]
    demo: bool,

    /// Write one frame to this PNG and exit: the state `--at` seconds into the replay
    /// (after `--from`) or the demo, computed without waiting; or, reading CAN, after
    /// listening for `--at` seconds.
    #[arg(long)]
    png: Option<PathBuf>,

    /// With `--png`: the moment to draw, in seconds.
    #[arg(long, default_value_t = 20.0, requires = "png")]
    at: f64,

    /// Write the demo's 60 s as a `candump -l` log and exit, to play onto a bus with
    /// `canplayer -I <file> vcan0=can0`.
    #[arg(long)]
    write_demo: Option<PathBuf>,

    /// Show in a desktop window instead of on the framebuffer.
    #[cfg(feature = "window")]
    #[arg(long)]
    window: bool,

    /// Frame size for `--png` and `--window`; the framebuffer uses its own.
    #[arg(long, default_value = "800x480", value_parser = parse_size)]
    size: (u32, u32),

    /// Framebuffer device.
    #[arg(long, default_value = "/dev/fb0")]
    fb_device: String,

    /// Frames per second.
    #[arg(long, default_value_t = 25.0)]
    fps: f64,

    /// Wing calibrations and boat geometry; the built-in boat.toml when omitted.
    #[arg(long)]
    boat: Option<PathBuf>,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WIDTHxHEIGHT")?;
    let w = w.parse().map_err(|_| "bad width")?;
    let h = h.parse().map_err(|_| "bad height")?;
    Ok((w, h))
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    let cfg = match &args.boat {
        Some(path) => BoatConfig::load(path)?,
        None => BoatConfig::default(),
    };
    let fonts = Fonts::new();

    if let Some(path) = &args.write_demo {
        return write_demo(path);
    }
    if let Some(png) = &args.png {
        return snapshot(&args, cfg, &fonts, png);
    }

    let model: Shared = Arc::new(Mutex::new(Model::new(cfg)));
    let clock = Instant::now();
    if args.demo {
        source::spawn_demo(model.clone(), clock);
    } else if let Some(path) = &args.replay {
        source::spawn_replay(model.clone(), clock, path, args.from, args.speed);
    } else {
        source::spawn_can(model.clone(), clock, &args.can_interface)
            .map_err(|e| format!("{}: {e}", args.can_interface))?;
    }
    let period = Duration::from_secs_f64(1.0 / args.fps);
    let mut stats = FrameStats::default();
    let mut paint = |pm: &mut Pixmap| {
        let started = Instant::now();
        let (view, history, cfg, now) = {
            let mut m = model.lock().unwrap();
            let now = clock.elapsed().as_secs_f64();
            (m.frame(now), m.history.clone(), m.cfg.clone(), now)
        };
        render::draw(pm, &fonts, &cfg, &view, &history, now);
        stats.add(started.elapsed());
    };

    #[cfg(feature = "window")]
    if args.window {
        let (w, h) = args.size;
        return window::run(w, h, period, paint);
    }

    let mut fb = eoi_foil_view::fb::Fb::open(&args.fb_device)?;
    let (w, h) = fb.size();
    let mut pm = Pixmap::new(w, h).ok_or("bad framebuffer size")?;
    loop {
        let started = Instant::now();
        paint(&mut pm);
        fb.show(&pm);
        if let Some(rest) = period.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

/// Feeds the source up to `--at`, draws once and saves.
fn snapshot(args: &Args, cfg: BoatConfig, fonts: &Fonts, png: &PathBuf) -> Result<(), String> {
    let mut model = Model::new(cfg);
    let mut at = args.at;
    if let Some(path) = &args.replay {
        let frames = source::candump_frames(path, args.from)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for (t, frame) in frames.take_while(|(t, _)| *t <= args.at) {
            source::apply(&mut model, &frame, t);
        }
    } else if args.demo {
        source::run_demo_until(&mut model, 0.0, args.at);
    } else {
        let shared: Shared = Arc::new(Mutex::new(model));
        let clock = Instant::now();
        source::spawn_can(shared.clone(), clock, &args.can_interface)
            .map_err(|e| format!("{}: {e}", args.can_interface))?;
        std::thread::sleep(Duration::from_secs_f64(args.at));
        at = clock.elapsed().as_secs_f64();
        model = shared.lock().unwrap().clone();
    }
    let view = model.frame(at);
    let (w, h) = args.size;
    let mut pm = Pixmap::new(w, h).ok_or("bad size")?;
    render::draw(&mut pm, fonts, &model.cfg, &view, &model.history, at);
    pm.save_png(png)
        .map_err(|e| format!("{}: {e}", png.display()))?;
    tracing::info!("wrote {}", png.display());
    Ok(())
}

fn write_demo(path: &PathBuf) -> Result<(), String> {
    use std::io::Write;
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = std::io::BufWriter::new(file);
    // An arbitrary epoch, so the log looks like any other candump.
    let epoch = 1_800_000_000.0;
    for tick in 0..600 {
        let t = f64::from(tick) * 0.1;
        for frame in source::demo_frames(t) {
            let id = match frame.id {
                embedded_can::Id::Standard(id) => format!("{:03X}", id.as_raw()),
                embedded_can::Id::Extended(id) => format!("{:08X}", id.as_raw()),
            };
            let data: String = frame.data.iter().map(|b| format!("{b:02X}")).collect();
            writeln!(out, "({:.6}) can0 {id}#{data}", epoch + t)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    out.flush()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    tracing::info!("wrote {}", path.display());
    Ok(())
}

/// Render times, logged every 10 s, to see what a frame costs on the Pi.
#[derive(Default)]
struct FrameStats {
    since: Option<Instant>,
    count: u32,
    total: Duration,
    worst: Duration,
}

impl FrameStats {
    fn add(&mut self, took: Duration) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.count += 1;
        self.total += took;
        self.worst = self.worst.max(took);
        if since.elapsed() >= Duration::from_secs(10) {
            tracing::info!(
                "{} frames, render {:.1} ms average, {:.1} ms worst",
                self.count,
                self.total.as_secs_f64() * 1e3 / f64::from(self.count),
                self.worst.as_secs_f64() * 1e3
            );
            *self = Self::default();
        }
    }
}
