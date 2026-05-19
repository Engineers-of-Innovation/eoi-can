mod candump;
mod filter;
mod flatten;
mod sampler;

use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::Parser;
use csv::WriterBuilder;
use eoi_can_decoder::{can_frame::CanFrame, parse_eoi_can_data};
use tracing::{info, warn};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::prelude::*;

use crate::filter::Filter;
use crate::sampler::{Mode, Sampler};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Convert a candump .log into a CSV by sampling decoded CAN data",
    long_about = "Convert a candump .log into a CSV by sampling decoded CAN data.\n\
                  \n\
                  Device selectors: all, battery, vesc, throttle, mppt, mppt:N (0..7),\n\
                  gan-mppt, gan-mppt:N (0..15), gnss, rudder, height, temperature.\n\
                  Combine via comma or repeated -d."
)]
struct Args {
    /// candump .log file to read
    input: PathBuf,

    /// Output CSV file. Defaults to <INPUT>.csv next to the input.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Sampling interval. Humantime like "1s", "500ms", "100us".
    /// Use "frame" (or "0") for one row per decoded frame.
    #[arg(long, default_value = "1s")]
    interval: String,

    /// Devices to export. Comma-separated and/or repeated.
    /// Examples: -d battery,gnss   -d mppt:1 -d mppt:3   -d all
    #[arg(short = 'd', long = "devices", required = true, num_args = 1..)]
    devices: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    register_tracing_subscriber(LevelFilter::INFO);
    let args = Args::parse();

    let filter = Filter::parse(&args.devices)?;
    let schema = flatten::column_schema(&filter);
    if schema.is_empty() {
        return Err(anyhow!("filter produced an empty column schema"));
    }

    let mode = parse_mode(&args.interval)?;
    let output_path = output_path(&args.input, args.output.as_deref());

    info!(
        input = %args.input.display(),
        output = %output_path.display(),
        mode = ?mode,
        columns = schema.len(),
        "starting conversion"
    );

    let input = File::open(&args.input)
        .with_context(|| format!("opening input {}", args.input.display()))?;
    let reader = BufReader::new(input);

    let output = File::create(&output_path)
        .with_context(|| format!("creating output {}", output_path.display()))?;
    let csv_writer = WriterBuilder::new().from_writer(BufWriter::new(output));

    let mut sampler = Sampler::new(mode, schema, csv_writer)?;

    let mut fields = Vec::with_capacity(16);
    let mut lines = 0u64;
    let mut decoded = 0u64;
    let mut filtered = 0u64;
    let mut undecoded = 0u64;

    for line in reader.lines() {
        let line = line.context("reading input line")?;
        lines += 1;

        let Some(raw) = candump::parse_line(&line) else {
            continue;
        };
        let frame = CanFrame::from_encoded(raw.id, &raw.data);
        let Some(data) = parse_eoi_can_data(&frame) else {
            undecoded += 1;
            continue;
        };
        decoded += 1;
        if !filter.accepts(&data) {
            continue;
        }
        filtered += 1;
        flatten::flatten(&data, &mut fields);
        sampler.accept(raw.timestamp_secs, &fields)?;
    }

    sampler.finish()?;

    info!(
        lines,
        decoded,
        kept = filtered,
        undecoded,
        "conversion complete"
    );
    if undecoded > 0 {
        warn!(
            "{undecoded} frames could not be decoded \
             (CAN IDs not recognised by eoi-can-decoder)"
        );
    }

    Ok(())
}

fn parse_mode(s: &str) -> anyhow::Result<Mode> {
    let trimmed = s.trim();
    if trimmed == "frame" || trimmed == "0" {
        return Ok(Mode::PerFrame);
    }
    let dur: Duration = humantime::parse_duration(trimmed)
        .with_context(|| format!("invalid --interval {trimmed:?}"))?;
    let secs = dur.as_secs_f64();
    if secs <= 0.0 {
        return Err(anyhow!("--interval must be positive or 'frame'"));
    }
    Ok(Mode::Bucketed {
        interval_secs: secs,
    })
}

fn output_path(input: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    let mut name: OsString = input.as_os_str().to_owned();
    name.push(".csv");
    PathBuf::from(name)
}

fn register_tracing_subscriber(level_filter: LevelFilter) {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(level_filter.into())
                .from_env_lossy(),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}
