use clap::Parser;
use embedded_can::{ExtendedId, Frame};
use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use embedded_graphics_simulator::{OutputSettingsBuilder, SimulatorDisplay, Window};
use eoi_can_decoder::parse_eoi_can_data;
use socketcan::{tokio::CanSocket, CanFrame};
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
                .with_default_directive("info".parse().unwrap())
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
            } else {
                warn!("Failed to parse data from CAN frame: {:?}", embedded_frame);
            }
        }
    });

    let mut display: SimulatorDisplay<BinaryColor> = SimulatorDisplay::new(Size::new(800, 480));

    draw_display::draw_display(&mut display)?;

    let output_settings = OutputSettingsBuilder::new().scale(1).max_fps(1).build();
    let mut window = Window::new(
        "Engineers of Innovation CAN Display Simulator",
        &output_settings,
    );

    window.show_static(&display);
    // window.update(&display);
    // sleep(Duration::from_secs(1)).await;

    Ok(())
}
