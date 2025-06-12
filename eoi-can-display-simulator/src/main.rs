use std::time::Duration;

use clap::Parser;
use embedded_can::Frame;
use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use embedded_graphics_simulator::{
    OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use eoi_can_decoder::{can_collector, parse_eoi_can_data};
use get_wifi_ip::get_wifi_ip;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn, Level};
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
async fn main() -> Result<(), core::convert::Infallible> {
    register_tracing_subscriber(LevelFilter::DEBUG);
    let args = Args::parse();
    info!("CAN interface: {}", args.can_interface);

    let can_sock: socketcan::tokio::AsyncCanSocket<socketcan::CanSocket> =
        socketcan::tokio::AsyncCanSocket::open(args.can_interface.as_str())
            .expect("Unable to open CAN socket");
    info!("Connected to CAN interface: {}", args.can_interface);

    let shared_can_collector = Arc::new(Mutex::new(can_collector::CanCollector::new()));

    let can_collector_receiver = shared_can_collector.clone();

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

    // Start displaying the data
    let mut display: SimulatorDisplay<BinaryColor> = SimulatorDisplay::new(Size::new(800, 480));
    let output_settings = OutputSettingsBuilder::new().scale(2).max_fps(10).build();
    let mut window = Window::new(
        "Engineers of Innovation CAN Display Simulator",
        &output_settings,
    );

    let mut display_data = draw_display::DisplayData::default();

    draw_display::draw_display(&mut display, &display_data).unwrap();

    tokio::time::sleep(Duration::from_millis(1000)).await; // load CAN data
    let mut last_time_updated_display = Instant::now() - Duration::from_secs(100);

    'running: loop {
        // Check if we have new CAN frames to process
        if last_time_updated_display.elapsed() > Duration::from_millis(100) {
            last_time_updated_display = Instant::now();
            if let Ok(mut can_collector) = shared_can_collector.lock() {
                if can_collector.get_dropped_frames() > 0 {
                    debug!("Dropped frames: {}", can_collector.get_dropped_frames());
                }
                let mut parsed_frames = 0_u32;
                can_collector.iter().for_each(|frame| {
                    if let Some(parsed_data) = parse_eoi_can_data(frame) {
                        display_data.ingest_eoi_can_data(parsed_data);
                        parsed_frames = parsed_frames.saturating_add(1);
                    } else {
                        warn!("Failed to parse data from CAN frame: {:?}", frame);
                    }
                });
                debug!("Parsed frames: {}", parsed_frames);
                can_collector.clear();
            }

            if let Some(ip) = get_wifi_ip() {
                display_data.ip_address.update(ip);
            }

            draw_display::draw_display(&mut display, &display_data).unwrap();
            window.update(&display);
        }

        for event in window.events() {
            if let SimulatorEvent::Quit = event {
                warn!("Received quit event, exiting...");
                break 'running;
            } else {
                trace!("Event: {:?}", event);
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await; // prevent hot loop
    }

    Ok(())
}
