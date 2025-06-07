use clap::Parser;
use embedded_can::Frame;
use eoi_can_decoder::{can_collector, parse_eoi_can_data};
use json_patch::merge;
use paho_mqtt as mqtt;
use serde_json::json;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[allow(unused_imports)]
use tracing::{Level, debug, error, info, trace, warn};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::prelude::*;

mod mqtt_settings;

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
                .with_default_directive(level_filter.into())
                .from_env_lossy(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_file(true)
                .with_line_number(true),
        )
        .init();
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

    let mut trust_store = env::current_dir().unwrap();
    trust_store.push(mqtt_settings::TRUST_STORE);

    if !trust_store.exists() {
        panic!("The trust store file does not exist: {:?}", trust_store);
    }

    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(mqtt_settings::BROKER.to_string())
        .client_id(mqtt_settings::CLIENT.to_string())
        .finalize();

    let client = mqtt::Client::new(create_opts).unwrap_or_else(|err| {
        panic!("Error creating the client: {:?}", err);
    });

    let ssl_opts = mqtt::SslOptionsBuilder::new()
        .trust_store(trust_store)
        .unwrap()
        .finalize();

    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .ssl_options(ssl_opts)
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .user_name(mqtt_settings::USER.to_string())
        .password(mqtt_settings::PASSWORD.to_string())
        .finalize();

    if let Err(error) = client.connect(conn_opts) {
        panic!("Unable to connect to MQTT broker: {:?}", error);
    }

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

    tokio::time::sleep(Duration::from_secs(1)).await;

    loop {
        if let Ok(mut can_collector) = shared_can_collector.lock() {
            if can_collector.get_dropped_frames() > 0 {
                trace!("Dropped frames: {}", can_collector.get_dropped_frames());
            }
            let mut parsed_frames = 0_u32;
            let mut merged_json = json!({});

            can_collector.iter().for_each(|frame| {
                trace!("Paring CAN frame: {:?}", frame);
                if let Some(data) = parse_eoi_can_data(frame) {
                    trace!("{:?}", data);
                    if let Ok(json) = serde_json::to_value(&data) {
                        trace!("{:?}", json);
                        merge(&mut merged_json, &json);
                    } else {
                        warn!("Failed to serialize json of {:?}", data)
                    }
                } else {
                    warn!("Failed to parse data from CAN frame: {:?}", frame);
                }
                parsed_frames = parsed_frames.saturating_add(1);
            });
            trace!("Parsed frames: {}", parsed_frames);
            can_collector.clear();

            // Send merged JSON to MQTT
            let mqtt_message = mqtt::Message::new(
                mqtt_settings::TOPIC.to_string(),
                merged_json.to_string(),
                mqtt::QOS_1,
            );
            if let Err(e) = client.publish(mqtt_message) {
                error!("Failed to publish message: {:?}", e);
            } else {
                debug!("Published message: {:?}", merged_json);
            }
        }

        // if let Some(ip) = get_wifi_ip() {
        // display_data.ip_address.update(ip);
        // }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
