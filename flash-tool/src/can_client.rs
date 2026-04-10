use can_socket::tokio::CanSocket;
use can_socket::{CanFrame, CanId, StandardId};
use std::time::Duration;
use tokio::time::timeout;

/// CAN IDs matching the bootloader protocol
const CAN_ID_HOST_TO_DEVICE: u16 = 0x030;
const CAN_ID_DEVICE_TO_HOST: u16 = 0x031;
const CAN_ID_WRITE_DATA: u16 = 0x032;

/// Message types (byte 0 of CAN command/response frames)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    GetState = 0x01,
    EraseApp = 0x02,
    WriteAck = 0x03,
    ValidateApp = 0x04,
    BootApp = 0x05,
    Reboot = 0x06,
    Error = 0xFF,
}

// Timeouts
const READ_TIMEOUT: Duration = Duration::from_millis(500);
const ERASE_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootloaderState {
    WaitingWithoutApp,
    WaitingWithApp,
    FlashingApp,
    Unknown(u8),
}

impl std::fmt::Display for BootloaderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingWithoutApp => write!(f, "WaitingWithoutApp"),
            Self::WaitingWithApp => write!(f, "WaitingWithApp"),
            Self::FlashingApp => write!(f, "FlashingApp"),
            Self::Unknown(v) => write!(f, "Unknown(0x{:02X})", v),
        }
    }
}

impl From<u8> for BootloaderState {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::WaitingWithoutApp,
            1 => Self::WaitingWithApp,
            2 => Self::FlashingApp,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationResult {
    Valid,
    BadMagic,
    BadLength,
    BadCrc,
    Unknown(u8),
}

impl std::fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Valid => write!(f, "Valid"),
            Self::BadMagic => write!(f, "BadMagic"),
            Self::BadLength => write!(f, "BadLength"),
            Self::BadCrc => write!(f, "BadCrc"),
            Self::Unknown(v) => write!(f, "Unknown(0x{:02X})", v),
        }
    }
}

impl From<u8> for ValidationResult {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Valid,
            1 => Self::BadMagic,
            2 => Self::BadLength,
            3 => Self::BadCrc,
            other => Self::Unknown(other),
        }
    }
}

pub struct CanClient {
    socket: CanSocket,
}

impl CanClient {
    pub async fn connect(interface: &str) -> Result<Self, ClientError> {
        let socket = CanSocket::bind(interface).map_err(ClientError::Bind)?;
        Ok(Self { socket })
    }

    /// Send a command frame and wait for a response with the expected type.
    async fn send_and_recv(
        &self,
        msg: MsgType,
        recv_timeout: Duration,
    ) -> Result<Vec<u8>, ClientError> {
        let id = CanId::Standard(StandardId::new(CAN_ID_HOST_TO_DEVICE).unwrap());
        let frame = make_frame(id, &[msg as u8]);
        self.socket.send(&frame).await.map_err(ClientError::Send)?;
        self.recv_expected(msg, recv_timeout).await
    }

    /// Wait for a response frame with the expected message type.
    async fn recv_expected(
        &self,
        expected: MsgType,
        recv_timeout: Duration,
    ) -> Result<Vec<u8>, ClientError> {
        let deadline = tokio::time::Instant::now() + recv_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(ClientError::Timeout);
            }
            let response = timeout(remaining, self.socket.recv())
                .await
                .map_err(|_| ClientError::Timeout)?
                .map_err(ClientError::Recv)?;

            // Filter for our response ID
            let resp_std_id = match response.id().as_standard() {
                Some(id) => id,
                None => continue,
            };
            if resp_std_id.as_u16() != CAN_ID_DEVICE_TO_HOST {
                continue;
            }
            // RTR frames have no data
            let resp_data = match response.data() {
                Some(data) => data,
                None => continue,
            };
            if resp_data.is_empty() {
                continue;
            }
            // Check for error response
            if resp_data[0] == MsgType::Error as u8 {
                let code = if resp_data.len() > 1 { resp_data[1] } else { 0 };
                return Err(ClientError::DeviceError(code));
            }
            if resp_data[0] == expected as u8 {
                return Ok(resp_data.to_vec());
            }
            // Ignore other response types (e.g. state broadcasts)
        }
    }

    pub async fn get_state(&self) -> Result<BootloaderState, ClientError> {
        let resp = self
            .send_and_recv(MsgType::GetState, READ_TIMEOUT)
            .await?;
        let state = resp.get(1).copied().unwrap_or(0);
        Ok(BootloaderState::from(state))
    }

    pub async fn erase_app(&self) -> Result<(), ClientError> {
        self.send_and_recv(MsgType::EraseApp, ERASE_TIMEOUT)
            .await?;
        Ok(())
    }

    pub async fn write_firmware(&self, data: &[u8]) -> Result<(), ClientError> {
        // Pad data to 8-byte alignment (0xFF padding matches erased flash)
        let padded_len = (data.len() + 7) & !7;
        let mut padded = vec![0xFF_u8; padded_len];
        padded[..data.len()].copy_from_slice(data);

        let total = padded.len();
        let mut offset = 0;
        while offset < total {
            let chunk = &padded[offset..offset + 8];

            // Send on dedicated write data CAN ID — all 8 bytes are payload
            let id = CanId::Standard(StandardId::new(CAN_ID_WRITE_DATA).unwrap());
            let frame = make_frame(id, chunk);
            self.socket.send(&frame).await.map_err(ClientError::Send)?;

            // Wait for write acknowledgment on the response channel
            let resp = self
                .recv_expected(MsgType::WriteAck, WRITE_TIMEOUT)
                .await?;

            // Parse offset acknowledgment
            if resp.len() < 5 {
                return Err(ClientError::BadResponse("WriteAck too short".into()));
            }
            let ack_offset =
                u32::from_le_bytes([resp[1], resp[2], resp[3], resp[4]]) as usize;
            let expected_offset = offset + 8;
            if ack_offset != expected_offset {
                return Err(ClientError::OffsetMismatch {
                    expected: expected_offset,
                    actual: ack_offset,
                });
            }

            offset += 8;

            // Print progress every 4KB
            if offset % 4096 == 0 || offset == total {
                let pct = (offset as f64 / total as f64 * 100.0) as u32;
                log::info!("Writing: {}/{} bytes ({}%)", offset, total, pct);
            }
        }
        Ok(())
    }

    pub async fn validate_app(&self) -> Result<ValidationResult, ClientError> {
        let resp = self
            .send_and_recv(MsgType::ValidateApp, READ_TIMEOUT)
            .await?;
        let result = resp.get(1).copied().unwrap_or(0xFF);
        Ok(ValidationResult::from(result))
    }

    pub async fn boot_app(&self) -> Result<(), ClientError> {
        self.send_and_recv(MsgType::BootApp, READ_TIMEOUT)
            .await?;
        Ok(())
    }

    pub async fn reboot(&self) -> Result<(), ClientError> {
        self.send_and_recv(MsgType::Reboot, READ_TIMEOUT)
            .await?;
        Ok(())
    }
}

/// Create a CanFrame from a dynamically-sized slice (up to 8 bytes).
fn make_frame(id: CanId, data: &[u8]) -> CanFrame {
    let mut buf = [0u8; 8];
    let len = data.len().min(8);
    buf[..len].copy_from_slice(&data[..len]);
    // CanData requires From<&[u8; N]> with fixed sizes.
    match len {
        0 => CanFrame::new(id, <&[u8; 0]>::try_from(&buf[..0]).unwrap()),
        1 => CanFrame::new(id, <&[u8; 1]>::try_from(&buf[..1]).unwrap()),
        2 => CanFrame::new(id, <&[u8; 2]>::try_from(&buf[..2]).unwrap()),
        3 => CanFrame::new(id, <&[u8; 3]>::try_from(&buf[..3]).unwrap()),
        4 => CanFrame::new(id, <&[u8; 4]>::try_from(&buf[..4]).unwrap()),
        5 => CanFrame::new(id, <&[u8; 5]>::try_from(&buf[..5]).unwrap()),
        6 => CanFrame::new(id, <&[u8; 6]>::try_from(&buf[..6]).unwrap()),
        7 => CanFrame::new(id, <&[u8; 7]>::try_from(&buf[..7]).unwrap()),
        _ => CanFrame::new(id, &buf),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Failed to bind CAN socket: {0}")]
    Bind(std::io::Error),
    #[error("Failed to send CAN frame: {0}")]
    Send(std::io::Error),
    #[error("Failed to receive CAN frame: {0}")]
    Recv(std::io::Error),
    #[error("Timeout waiting for response")]
    Timeout,
    #[error("Device reported error: 0x{0:02X}")]
    DeviceError(u8),
    #[error("Bad response: {0}")]
    BadResponse(String),
    #[error("Write offset mismatch: expected {expected}, got {actual}")]
    OffsetMismatch { expected: usize, actual: usize },
}
