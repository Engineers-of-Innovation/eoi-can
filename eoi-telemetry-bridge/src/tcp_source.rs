use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;

use crate::candump::parse_line;
use crate::live_state::{apply_frame, LiveState};

/// Connects to a portable frame source — `eoi-can-sim`'s TCP broadcaster, or anything
/// else that speaks the same candump-line format — and applies every frame as it
/// arrives. This is what makes live development work on macOS/Windows, where the
/// SocketCAN path in `live.rs` isn't available. Reconnects with a fixed backoff if
/// the source is unreachable or drops, same as `live::read_loop`.
pub async fn stream_loop(addr: &str, state: Arc<Mutex<LiveState>>) {
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                tracing::info!(addr, "tcp frame source connected");
                let mut lines = BufReader::new(stream).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            if let Some((_ts, id, data)) = parse_line(&line) {
                                apply_frame(&mut state.lock(), id, &data);
                            }
                        }
                        Ok(None) => {
                            tracing::warn!(addr, "tcp frame source closed");
                            break;
                        }
                        Err(e) => {
                            tracing::error!(addr, error = %e, "tcp frame source read failed");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(addr, error = %e, "failed to connect to tcp frame source");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
