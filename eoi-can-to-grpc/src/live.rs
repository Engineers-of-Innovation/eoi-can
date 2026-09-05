use std::sync::Arc;

use embedded_can::{Frame, Id};
use parking_lot::Mutex;
use socketcan::CanFrame;

use crate::live_state::{apply_frame, LiveState};

/// Reads frames from a real (or virtual) SocketCAN interface forever, applying each
/// to `state`. Reconnects with a fixed backoff if the interface disappears — this is
/// the path a flaky CAN adapter or a `vcan0` brought up after the bridge starts will hit.
pub async fn read_loop(iface: &str, state: Arc<Mutex<LiveState>>) {
    loop {
        match socketcan::tokio::AsyncCanSocket::<socketcan::CanSocket>::open(iface) {
            Ok(sock) => {
                tracing::info!(iface, "socketcan opened");
                loop {
                    match sock.read_frame().await {
                        Ok(CanFrame::Data(frame)) => {
                            let id = match frame.id() {
                                Id::Standard(s) => s.as_raw() as u32,
                                Id::Extended(e) => e.as_raw(),
                            };
                            apply_frame(&mut state.lock(), id, frame.data());
                        }
                        Ok(_) => {
                            // Remote/error frames carry no signal data.
                        }
                        Err(e) => {
                            tracing::error!(iface, error = %e, "socketcan read failed");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(iface, error = %e, "failed to open socketcan interface");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
