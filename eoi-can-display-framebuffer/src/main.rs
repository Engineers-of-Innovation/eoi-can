use core::time;

use clap::Parser;
use embedded_can::{ExtendedId, Frame};
use embedded_graphics_framebuffer::FrameBufferDisplay;
use eoi_can_decoder::{EoiCanData, parse_eoi_can_data};
use socketcan::{CanFrame, tokio::CanSocket};
use tokio::time::sleep;
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

    let can_sock: socketcan::tokio::AsyncCanSocket<socketcan::CanSocket> =
        CanSocket::open(args.can_interface.as_str()).expect("Unable to open CAN socket");
    info!("Connected to CAN interface: {}", args.can_interface);

    let (can_decoder_tx, mut can_decoder_rx) = tokio::sync::mpsc::channel::<EoiCanData>(100);

    // hack to check if the socket is open, not sure how to go about this
    can_sock
        .write_frame(CanFrame::new(ExtendedId::new(0).unwrap(), &[]).unwrap())
        .await
        .expect("Unable to write to CAN socket");

    // Spawn a task to read CAN frames
    tokio::spawn(async move {
        loop {
            let frame = can_sock.read_frame().await.unwrap();

            let embedded_frame = if let CanFrame::Data(frame) = frame {
                trace!(
                    "Received CAN frame: ID: {:?}, Data: {:?}",
                    frame.id(),
                    frame.data()
                );

                eoi_can_decoder::can_frame::CanFrame::from_encoded(frame.id(), frame.data())
            } else {
                continue;
            };

            let parsed_data = parse_eoi_can_data(&embedded_frame);
            if let Some(parsed) = parsed_data {
                info!("Parsed data: {:?}", parsed);
                if can_decoder_tx.send(parsed).await.is_err() {
                    warn!("Failed to send parsed data to display task");
                }
            } else {
                warn!("Failed to parse data from CAN frame: {:?}", embedded_frame);
            }
        }
    });

    let mut display = FrameBufferDisplay::new();
    display.flush().unwrap();

    let mut display_data = draw_display::DisplayData::default();
    draw_display::draw_display(&mut display, &display_data).unwrap();
    display.flush().unwrap();

    loop {
        // await for new data, otherwise we don't need to update the display
        if let Ok(Some(parsed_data)) =
            tokio::time::timeout(time::Duration::from_secs(1), can_decoder_rx.recv()).await
        {
            display_data.ingest_eoi_can_data(parsed_data);
        }
        // check if there are any other messages in the queue and process them right away
        while let Ok(parsed_data) = can_decoder_rx.try_recv() {
            display_data.ingest_eoi_can_data(parsed_data);
        }

        draw_display::draw_display(&mut display, &display_data).unwrap();
        display.flush().unwrap();

        sleep(time::Duration::from_millis(100)).await; // not to update too often
    }
}
