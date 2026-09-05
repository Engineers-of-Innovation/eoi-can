use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rand::Rng;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataValue;
use tonic::Request;

use crate::pb::eoi::telemetry::v1::telemetry_ingest_client::TelemetryIngestClient;
use crate::pb::eoi::telemetry::v1::Snapshot;
use crate::server::TelemetrySvc;

/// How many snapshots to hold in memory while the relay is unreachable. At 25 Hz
/// (the fastest allowed rate) this covers a little over 10 seconds of outage —
/// enough for a 4G blip, not a substitute for real backfill. Not persisted to
/// disk: a bridge restart still loses whatever was buffered.
const MAX_BUFFERED: usize = 256;
const MIN_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Pushes snapshots to the relay's `TelemetryIngest` service (implemented by the
/// `eoi-grpc-telemetry` repo's `eoi-telemetry-relay`). Samples on its own timer
/// regardless of connection state, buffering up to `MAX_BUFFERED` snapshots (oldest
/// dropped first) so a brief reconnect doesn't leave a gap in the relay's feed.
/// Reconnects with exponential backoff and jitter so a flapping link doesn't hammer
/// the relay.
///
/// `token`, if set, is sent as `authorization: Bearer <token>` on the push stream —
/// the same shape as `eoi-can-to-mqtt`'s username/password, minus the transport
/// encryption that gives that scheme its teeth, so this alone doesn't stop
/// eavesdropping, only unauthenticated pushes.
pub async fn push_loop(addr: String, token: Option<String>, svc: TelemetrySvc, period: Duration) {
    let buffer: Arc<Mutex<VecDeque<Snapshot>>> = Arc::new(Mutex::new(VecDeque::new()));

    let sample_buffer = buffer.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        loop {
            tick.tick().await;
            let mut buf = sample_buffer.lock();
            if buf.len() >= MAX_BUFFERED {
                buf.pop_front();
            }
            buf.push_back(svc.snapshot());
        }
    });

    let mut backoff = MIN_BACKOFF;
    loop {
        match TelemetryIngestClient::connect(addr.clone()).await {
            Ok(mut client) => {
                tracing::info!(addr, "connected to relay");
                backoff = MIN_BACKOFF;
                let (tx, rx) = mpsc::channel(4);
                let drain_buffer = buffer.clone();
                let sender = tokio::spawn(async move {
                    loop {
                        let next = drain_buffer.lock().pop_front();
                        match next {
                            Some(snap) => {
                                if tx.send(snap).await.is_err() {
                                    break;
                                }
                            }
                            None => tokio::time::sleep(Duration::from_millis(50)).await,
                        }
                    }
                });
                let mut request = Request::new(ReceiverStream::new(rx));
                if let Some(token) = &token {
                    match MetadataValue::try_from(format!("Bearer {token}")) {
                        Ok(value) => {
                            request.metadata_mut().insert("authorization", value);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "relay token is not a valid header value; sending unauthenticated")
                        }
                    }
                }
                if let Err(e) = client.push(request).await {
                    tracing::warn!(addr, error = %e, "ingest stream ended");
                }
                sender.abort();
            }
            Err(e) => {
                tracing::error!(addr, error = %e, "failed to connect to relay");
            }
        }
        let jitter = Duration::from_millis(rand::thread_rng().gen_range(0..250));
        tokio::time::sleep(backoff + jitter).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}
