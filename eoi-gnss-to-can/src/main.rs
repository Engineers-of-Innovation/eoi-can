use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use clap::Parser;
use embedded_can::{Frame, StandardId};
use gpsd_client::*;
use socketcan::{CanFrame, tokio::CanSocket};
use std::process;
use std::thread;
use std::time::Duration;
#[allow(unused_imports)]
use tracing::{Level, debug, error, info, trace, warn};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::prelude::*;
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// CAN interface
    #[arg(short, long, default_value_t = String::from("vcan0"))]
    can_interface: String,
}

fn register_tracing_subscriber(level_filter: LevelFilter) {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(level_filter.into())
                .from_env_lossy(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_file(true)
                .with_line_number(true),
        )
        .init();

    info!(
        "Environment override for debug level to: {:?}",
        level_filter
    );
}
#[tokio::main]
async fn main() {
    register_tracing_subscriber(LevelFilter::INFO);
    let args = Args::parse();
    info!("CAN interface: {}", args.can_interface);

    // Connecting to the gpsd socket server.
    let mut gps: GPS = match GPS::connect() {
        Ok(t) => t,
        Err(e) => {
            error!("{e}");
            process::exit(1);
        }
    };

    info!("Connected to gpsd server");

    let can_sock: socketcan::tokio::AsyncCanSocket<socketcan::CanSocket> =
        CanSocket::open(args.can_interface.as_str()).expect("Unable to open CAN socket");
    info!("Connected to CAN interface: {}", args.can_interface);

    loop {
        let data: GPSData = gps.current_data().unwrap();
        debug!("{data:#?}");

        let fix: u8 = matches!(data.mode, gpsd_client::Fix::Fix3D) as u8;
        let datetime: DateTime<Utc> = data.time.parse().expect("Invalid ISO8601 format");
        let datetime: DateTime<Local> = datetime.with_timezone(&Local);
        let hour: u8 = datetime.hour().try_into().unwrap();
        let minute: u8 = datetime.minute().try_into().unwrap();
        let second: u8 = datetime.second().try_into().unwrap();
        let year: u16 = datetime.year().try_into().unwrap();
        let month: u8 = datetime.month().try_into().unwrap();
        let day: u8 = datetime.day().try_into().unwrap();

        let can_block = [
            CanFrame::new(
                StandardId::new(0x200).unwrap(),
                &[fix, data.sats, data.sats_valid],
            )
            .unwrap(),
            CanFrame::new(
                StandardId::new(0x201).unwrap(),
                &data
                    .convert_speed(false) //kph
                    .to_le_bytes()
                    .iter()
                    .chain(data.track.to_le_bytes().iter())
                    .copied()
                    .collect::<Vec<u8>>(),
            )
            .unwrap(),
            CanFrame::new(StandardId::new(0x202).unwrap(), &data.lat.to_le_bytes()).unwrap(),
            CanFrame::new(StandardId::new(0x203).unwrap(), &data.lon.to_le_bytes()).unwrap(),
            CanFrame::new(
                StandardId::new(0x204).unwrap(),
                &year
                    .to_le_bytes()
                    .iter()
                    .chain(month.to_le_bytes().iter())
                    .chain(day.to_le_bytes().iter())
                    .chain(hour.to_le_bytes().iter())
                    .chain(minute.to_le_bytes().iter())
                    .chain(second.to_le_bytes().iter())
                    .copied()
                    .collect::<Vec<u8>>(),
            )
            .unwrap(),
        ];

        for frame in can_block.iter() {
            trace!("CAN frame: {:?}", frame);

            match can_sock.write_frame(*frame).await {
                Ok(_) => trace!("Sent CAN frame: {:?}", frame),
                Err(e) => error!("Failed to send CAN frame: {e}"),
            }
        }

        info!(
            "Fix: {fix}, Sats: {}, Sats Valid: {}",
            data.sats, data.sats_valid
        );
        info!(
            "Speed: {} kph, Track: {} degrees",
            data.convert_speed(false),
            data.track
        );
        info!("Latitude, Longitude: {},{}", data.lat, data.lon);
        info!("Time: {:02}:{:02}:{:02}", hour, minute, second);
        info!("Date: {:04}-{:02}-{:02}", year, month, day);

        thread::sleep(Duration::from_millis(1000));
    }
}
