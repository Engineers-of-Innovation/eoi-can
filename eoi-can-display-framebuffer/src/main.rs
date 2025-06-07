use clap::Parser;
use embedded_can::Frame;
use embedded_graphics_framebuffer::FrameBufferDisplay;
use eoi_can_decoder::{can_collector, parse_eoi_can_data};
use get_wifi_ip::get_wifi_ip;
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[allow(unused_imports)]
use tracing::{Level, debug, error, info, trace, warn};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::prelude::*;
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// CAN interface
    #[arg(short, long, default_value_t = String::from("can0"))]
    can_interface: String,
}

fn register_tracing_subscriber(level_filter: LevelFilter) {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive("debug".parse().unwrap())
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
async fn main() -> Result<(), core::convert::Infallible> {
    register_tracing_subscriber(LevelFilter::DEBUG);
    let args = Args::parse();
    info!("CAN interface: {}", args.can_interface);

    let shared_can_collector = Arc::new(Mutex::new(can_collector::CanCollector::new()));

    let can_collector_receiver = shared_can_collector.clone();

    let can_sock: socketcan::tokio::AsyncCanSocket<socketcan::CanSocket> =
        socketcan::tokio::AsyncCanSocket::open(args.can_interface.as_str())
            .expect("Unable to open CAN socket");
    info!("Connected to CAN interface: {}", args.can_interface);

    // Spawn a task to read CAN frames
    tokio::spawn(async move {
        loop {
            let frame = can_sock.read_frame().await.unwrap();

            let embedded_frame = if let socketcan::CanFrame::Data(frame) = frame {
                trace!(
                    "Received CAN frame: ID: {:?}, Data: {:?}",
                    frame.id(),
                    frame.data()
                );

                eoi_can_decoder::can_frame::CanFrame::from_encoded(frame.id(), frame.data())
            } else {
                debug!("Received non-data CAN frame: {:?}", frame);
                continue;
            };

            if let Ok(mut collector) = can_collector_receiver.lock() {
                collector.insert(embedded_frame);
            }
        }
    });

    let mut display = FrameBufferDisplay::new();
    display.flush().unwrap();

    let mut display_data = draw_display::DisplayData::default();
    draw_display::draw_display(&mut display, &display_data).unwrap();
    display.flush().unwrap();

    loop {
        if let Ok(mut can_collector) = shared_can_collector.lock() {
            if can_collector.get_dropped_frames() > 0 {
                trace!("Dropped frames: {}", can_collector.get_dropped_frames());
            }
            let mut parsed_frames = 0_u32;
            can_collector.iter().for_each(|frame| {
                trace!("Paring CAN frame: {:?}", frame);
                if let Some(parsed_data) = parse_eoi_can_data(frame) {
                    display_data.ingest_eoi_can_data(parsed_data);
                    parsed_frames = parsed_frames.saturating_add(1);
                } else {
                    warn!("Failed to parse data from CAN frame: {:?}", frame);
                }
            });
            trace!("Parsed frames: {}", parsed_frames);
            can_collector.clear();
        }

        if let Some(ip) = get_wifi_ip() {
            display_data.ip_address.update(ip);
        }

        draw_display::draw_display(&mut display, &display_data).unwrap();
        display.flush().unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await
    }
}
