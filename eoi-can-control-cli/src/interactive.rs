use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use eoi_can_decoder::ServoStatus;
use socketcan::tokio::CanSocket;
use tracing::info;

use crate::rudder::{
    SETPOINT_MAX, SETPOINT_MIN, decode_status, draw_status_line, initialize_frame, setpoint_frame,
};

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        println!();
    }
}

pub async fn run(socket: &CanSocket, step: u16) -> anyhow::Result<()> {
    info!(
        "←/→ nudge by {step}, Home/End jump to {SETPOINT_MIN}/{SETPOINT_MAX}, \
         i = initialize, q/Esc = quit"
    );

    // crossterm's event::read() blocks, so keys are pumped from a plain thread
    // into the async loop. The thread exits when the channel closes.
    let (key_tx, mut keys) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = crossterm::event::read() {
            if key_tx.send(event).is_err() {
                return;
            }
        }
    });

    let _raw_mode = RawModeGuard::new()?;
    let mut setpoint = SETPOINT_MIN;
    let mut status: Option<ServoStatus> = None;
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    draw_status_line(setpoint, status.as_ref())?;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                socket.write_frame(setpoint_frame(setpoint)).await?;
            }
            frame = socket.read_frame() => {
                if let Some(new_status) = decode_status(&frame?) {
                    status = Some(new_status);
                    draw_status_line(setpoint, status.as_ref())?;
                }
            }
            event = keys.recv() => {
                let Some(event) = event else {
                    anyhow::bail!("keyboard reader stopped");
                };
                let Event::Key(key) = event else { continue };
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Left => setpoint = setpoint.saturating_sub(step).max(SETPOINT_MIN),
                    KeyCode::Right => setpoint = setpoint.saturating_add(step).min(SETPOINT_MAX),
                    KeyCode::Home => setpoint = SETPOINT_MIN,
                    KeyCode::End => setpoint = SETPOINT_MAX,
                    KeyCode::Char('i') => socket.write_frame(initialize_frame()).await?,
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => continue,
                }
                draw_status_line(setpoint, status.as_ref())?;
            }
        }
    }
}
