mod candump;
mod ingest_client;
#[cfg(target_os = "linux")]
mod live;
mod live_state;
mod proto_map;
mod server;
mod tcp_source;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use live_state::LiveState;
use parking_lot::Mutex;
use tonic::transport::Server;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::pb::eoi::telemetry::v1::telemetry_server::TelemetryServer;
use crate::server::TelemetrySvc;

pub mod pb {
    pub mod eoi {
        pub mod telemetry {
            pub mod v1 {
                tonic::include_proto!("eoi.telemetry.v1");
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "eoi-telemetry-bridge")]
struct Args {
    /// A candump log (standard `(t) iface ID#data` format); a SocketCAN interface
    /// name (e.g. `can0`, `vcan0`, Linux only) to read live frames; or `tcp:<host:port>`
    /// to read the same candump-line format over TCP from a portable frame source such
    /// as `eoi-can-sim` (works on macOS/Windows too).
    #[arg(long)]
    source: String,

    /// Rewind the log when it ends so the UI stays live. Only meaningful for a
    /// candump-file source.
    #[arg(long)]
    r#loop: bool,

    /// Snapshot rate advertised as the default if the client sends hz=0.
    #[arg(long, default_value_t = 5)]
    hz: u32,

    #[arg(long, default_value = "127.0.0.1:50051")]
    bind: String,

    /// Relay address (e.g. `http://127.0.0.1:50060`) to push snapshots to over
    /// `TelemetryIngest`. Omit to run local-only.
    #[arg(long)]
    relay: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "eoi_telemetry_bridge=info".into()),
        )
        .init();

    let args = Args::parse();
    // For the local Telemetry server, a client's StreamStateRequest.hz is authoritative
    // and args.hz only documents the default; it's also reused below as the relay push rate.
    let state = Arc::new(Mutex::new(LiveState::new()));

    let path = PathBuf::from(&args.source);
    let session_id = if path.is_file() {
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session")
            .to_string();
        let replay_state = state.clone();
        let replay_path = path.clone();
        let looping = args.r#loop;
        tokio::spawn(async move {
            candump::replay_loop(&replay_path, looping, replay_state).await;
        });
        session_id
    } else if let Some(addr) = args.source.strip_prefix("tcp:") {
        let addr = addr.to_string();
        let tcp_state = state.clone();
        let session_addr = addr.clone();
        tokio::spawn(async move {
            tcp_source::stream_loop(&addr, tcp_state).await;
        });
        format!("tcp-{session_addr}")
    } else {
        // Not an existing file or `tcp:` source — treat it as a SocketCAN interface
        // name (e.g. `can0`, `vcan0`).
        #[cfg(target_os = "linux")]
        {
            let iface = args.source.clone();
            let live_state = state.clone();
            tokio::spawn(async move {
                live::read_loop(&iface, live_state).await;
            });
            args.source.clone()
        }
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!(
                "source is not a file or tcp: address, and SocketCAN is only available on Linux: {}",
                args.source
            );
            std::process::exit(2);
        }
    };

    let svc = TelemetrySvc {
        state,
        session_id,
        seq: Arc::new(Mutex::new(0)),
    };

    if let Some(relay_addr) = args.relay.clone() {
        let ingest_svc = svc.clone();
        let period = std::time::Duration::from_secs_f32(1.0 / args.hz.clamp(1, 25) as f32);
        tokio::spawn(async move {
            ingest_client::push_loop(relay_addr, ingest_svc, period).await;
        });
    }

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_credentials(false)
        .allow_headers(tower_http::cors::Any)
        .expose_headers(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any);

    let addr = args.bind.parse()?;
    tracing::info!(%addr, source = %args.source, "listening (grpc-web)");

    Server::builder()
        .accept_http1(true)
        .layer(cors)
        .layer(GrpcWebLayer::new())
        .add_service(TelemetryServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
