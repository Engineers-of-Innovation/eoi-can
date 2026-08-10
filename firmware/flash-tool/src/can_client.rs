use can_socket::tokio::CanSocket;
use can_socket::{CanFrame, CanId, StandardId};
use std::time::Duration;
use tokio::time::timeout;

use eoi_boot_api::header::AppType;
use eoi_boot_api::protocol::{
    self, BoardAddress, VersionInfo, app_type_from_resp_id, board_address,
};

// Timeouts
const READ_TIMEOUT: Duration = Duration::from_millis(500);
const ERASE_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long to collect discovery replies. Bootloaders answer immediately, so
/// this only has to cover one round trip plus a board that is mid-reboot.
const DISCOVERY_WINDOW: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootloaderState {
    WaitingWithoutApp,
    WaitingWithApp,
    FlashingApp,
    /// Not a bootloader state at all: the application answered. Anything that
    /// needs the bootloader has to reboot the board first.
    AppRunning,
    Unknown(u8),
}

impl BootloaderState {
    /// Whether the bootloader — rather than the application — is answering.
    pub fn in_bootloader(&self) -> bool {
        !matches!(self, Self::AppRunning | Self::Unknown(_))
    }
}

impl std::fmt::Display for BootloaderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingWithoutApp => write!(f, "WaitingWithoutApp"),
            Self::WaitingWithApp => write!(f, "WaitingWithApp"),
            Self::FlashingApp => write!(f, "FlashingApp"),
            Self::AppRunning => write!(f, "AppRunning"),
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
            protocol::STATE_APP_RUNNING => Self::AppRunning,
            other => Self::Unknown(other),
        }
    }
}

/// Render a [`VersionInfo`] the way every command prints it.
pub fn format_version(v: &VersionInfo) -> String {
    let git = if v.git_unknown {
        "unknown".to_string()
    } else {
        format!("{:02x}{:02x}{:02x}", v.git[0], v.git[1], v.git[2])
    };
    format!(
        "{} {}.{}.{} git {}{}",
        if v.bootloader { "boot" } else { "app" },
        v.major,
        v.minor,
        v.patch,
        git,
        if v.dirty { "-dirty" } else { "" }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationResult {
    Valid,
    BadMagic,
    BadLength,
    BadCrc,
    WrongAppType,
    Unknown(u8),
}

impl std::fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Valid => write!(f, "Valid"),
            Self::BadMagic => write!(f, "BadMagic"),
            Self::BadLength => write!(f, "BadLength"),
            Self::BadCrc => write!(f, "BadCrc"),
            Self::WrongAppType => write!(f, "WrongAppType"),
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
            4 => Self::WrongAppType,
            other => Self::Unknown(other),
        }
    }
}

/// State of one board, as reported on its own response ID.
#[derive(Debug, Clone, Copy)]
pub struct DeviceState {
    pub state: BootloaderState,
    /// App type the board reports in byte 2. `None` from a bootloader that
    /// predates the addressed protocol.
    pub app_type: Option<AppType>,
    /// Build identity of whatever answered. `None` from firmware that predates
    /// `GET_VERSION`.
    pub version: Option<VersionInfo>,
}

/// A client bound to one board's CAN address. Every frame it sends carries that
/// board's command or data ID, so it cannot touch another board.
pub struct CanClient {
    socket: CanSocket,
    addr: BoardAddress,
}

impl CanClient {
    pub async fn connect(interface: &str, app_type: AppType) -> Result<Self, ClientError> {
        let socket = CanSocket::bind(interface).map_err(ClientError::Bind)?;
        Ok(Self {
            socket,
            addr: board_address(app_type),
        })
    }

    /// Broadcast on the discovery ID and collect every board that answers.
    ///
    /// Each board replies on its own response ID, so replies are attributed by
    /// source ID rather than by a payload field — and no two boards ever
    /// transmit the same identifier, which would collide on the wire.
    ///
    /// Both questions go out up front and the answers are collected in one
    /// window: a board's state and its version arrive as two separate frames,
    /// and running applications answer alongside bootloaders.
    pub async fn discover(interface: &str) -> Result<Vec<DeviceState>, ClientError> {
        let socket = CanSocket::bind(interface).map_err(ClientError::Bind)?;
        let id = CanId::Standard(StandardId::new(protocol::CAN_ID_DISCOVERY).unwrap());
        for msg in [protocol::msg::GET_STATE, protocol::msg::GET_VERSION] {
            socket
                .send(&make_frame(id, &[msg]))
                .await
                .map_err(ClientError::Send)?;
        }

        // States and versions arrive as separate frames in any order, so they
        // are collected apart and merged by `merge_discovery` once the window
        // closes.
        let mut states: Vec<(AppType, BootloaderState)> = Vec::new();
        let mut versions: Vec<(AppType, VersionInfo)> = Vec::new();
        let deadline = tokio::time::Instant::now() + DISCOVERY_WINDOW;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(Ok(response)) = timeout(remaining, socket.recv()).await else {
                break;
            };
            let Some(resp_id) = response.id().as_standard() else {
                continue;
            };
            let Some(app_type) = app_type_from_resp_id(resp_id.as_u16()) else {
                continue;
            };
            let Some(data) = response.data() else {
                continue;
            };

            match data.first() {
                Some(&protocol::msg::GET_STATE) => {
                    // Byte 2 is the board's own view of its app type. The source ID is
                    // authoritative; a mismatch means a mis-flashed bootloader.
                    let reported = data.get(2).copied().and_then(AppType::from_u8);
                    if let Some(reported) = reported
                        && reported != app_type
                    {
                        log::warn!(
                            "Board on 0x{:03X} reports app type {:?} but that ID belongs to {:?}",
                            resp_id.as_u16(),
                            reported,
                            app_type
                        );
                    }
                    let state = BootloaderState::from(data.get(1).copied().unwrap_or(0));
                    if !states.iter().any(|(t, _)| *t == app_type) {
                        states.push((app_type, state));
                    }
                }
                Some(&protocol::msg::GET_VERSION) => {
                    let Some(version) = VersionInfo::decode(&data) else {
                        continue;
                    };
                    if !versions.iter().any(|(t, _)| *t == app_type) {
                        versions.push((app_type, version));
                    }
                }
                // Errors and stray responses (a bootloader that predates
                // GET_VERSION answers with one) say nothing about the board.
                _ => continue,
            }
        }

        Ok(merge_discovery(&states, &versions))
    }

    /// Send a command frame and wait for a response with the expected type.
    async fn send_and_recv(&self, msg: u8, recv_timeout: Duration) -> Result<Vec<u8>, ClientError> {
        let id = CanId::Standard(StandardId::new(self.addr.cmd).unwrap());
        let frame = make_frame(id, &[msg]);
        self.socket.send(&frame).await.map_err(ClientError::Send)?;
        self.recv_expected(msg, recv_timeout).await
    }

    /// Wait for a response frame with the expected message type.
    async fn recv_expected(
        &self,
        expected: u8,
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
            if resp_std_id.as_u16() != self.addr.resp {
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
            if resp_data[0] == protocol::msg::ERROR {
                let code = if resp_data.len() > 1 { resp_data[1] } else { 0 };
                return Err(ClientError::DeviceError(code));
            }
            if resp_data[0] == expected {
                return Ok(resp_data.to_vec());
            }
            // Ignore other response types (e.g. state broadcasts)
        }
    }

    pub async fn get_state(&self) -> Result<DeviceState, ClientError> {
        let resp = self
            .send_and_recv(protocol::msg::GET_STATE, READ_TIMEOUT)
            .await?;
        Ok(DeviceState {
            state: BootloaderState::from(resp.get(1).copied().unwrap_or(0)),
            app_type: resp.get(2).copied().and_then(AppType::from_u8),
            version: None,
        })
    }

    /// Build identity of whatever is answering — bootloader or application.
    ///
    /// `Ok(None)` means the board is there but does not implement the command:
    /// a bootloader flashed before `GET_VERSION` existed rejects it, and it can
    /// only be replaced over SWD, so that is a fact to report rather than fail on.
    pub async fn get_version(&self) -> Result<Option<VersionInfo>, ClientError> {
        match self
            .send_and_recv(protocol::msg::GET_VERSION, READ_TIMEOUT)
            .await
        {
            Ok(resp) => Ok(VersionInfo::decode(&resp)),
            Err(ClientError::DeviceError(protocol::err::UNKNOWN_COMMAND)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn erase_app(&self) -> Result<(), ClientError> {
        self.send_and_recv(protocol::msg::ERASE_APP, ERASE_TIMEOUT)
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

            // Send on this board's dedicated write data ID — all 8 bytes are payload
            let id = CanId::Standard(StandardId::new(self.addr.data).unwrap());
            let frame = make_frame(id, chunk);
            self.socket.send(&frame).await.map_err(ClientError::Send)?;

            // Wait for write acknowledgment on the response channel
            let resp = self
                .recv_expected(protocol::msg::WRITE_ACK, WRITE_TIMEOUT)
                .await?;

            // Parse offset acknowledgment
            if resp.len() < 5 {
                return Err(ClientError::BadResponse("WriteAck too short".into()));
            }
            let ack_offset = u32::from_le_bytes([resp[1], resp[2], resp[3], resp[4]]) as usize;
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
            .send_and_recv(protocol::msg::VALIDATE_APP, READ_TIMEOUT)
            .await?;
        let result = resp.get(1).copied().unwrap_or(0xFF);
        Ok(ValidationResult::from(result))
    }

    pub async fn boot_app(&self) -> Result<(), ClientError> {
        self.send_and_recv(protocol::msg::BOOT_APP, READ_TIMEOUT)
            .await?;
        Ok(())
    }

    pub async fn reboot(&self) -> Result<(), ClientError> {
        self.send_and_recv(protocol::msg::REBOOT, READ_TIMEOUT)
            .await?;
        Ok(())
    }
}

/// Combine the state and version replies collected during one discovery window.
///
/// A board answers the two broadcasts with two separate frames that can arrive
/// in either order, and either can be lost on a busy bus. Split out so the
/// pairing is testable without a socket.
fn merge_discovery(
    states: &[(AppType, BootloaderState)],
    versions: &[(AppType, VersionInfo)],
) -> Vec<DeviceState> {
    // State replies first, so the table keeps the order it has always had.
    let mut found: Vec<DeviceState> = states
        .iter()
        .map(|(app_type, state)| DeviceState {
            state: *state,
            app_type: Some(*app_type),
            version: versions
                .iter()
                .find(|(t, _)| t == app_type)
                .map(|(_, v)| *v),
        })
        .collect();
    for (app_type, version) in versions {
        if states.iter().any(|(t, _)| t == app_type) {
            continue;
        }
        // Version but no state: firmware that answers GetVersion also answers
        // GetState, so this is a dropped frame rather than an old image. Report
        // what is known instead of hiding the board.
        found.push(DeviceState {
            state: BootloaderState::Unknown(0),
            app_type: Some(*app_type),
            version: Some(*version),
        });
    }
    found
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
        _ => CanFrame::new(id, buf),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn version(bootloader: bool, dirty: bool, git_unknown: bool) -> VersionInfo {
        VersionInfo {
            major: 0,
            minor: 1,
            patch: 0,
            git: [0x7E, 0x21, 0xB0],
            dirty,
            bootloader,
            git_unknown,
        }
    }

    /// State 3 must decode to `AppRunning` and read as "not in the bootloader",
    /// because that is what stops `flash` erasing under a live application.
    #[test]
    fn state_byte_three_is_a_running_application() {
        assert_eq!(BootloaderState::from(3), BootloaderState::AppRunning);
        assert!(!BootloaderState::from(3).in_bootloader());
        for byte in 0..=2u8 {
            assert!(
                BootloaderState::from(byte).in_bootloader(),
                "state {byte} is a bootloader state"
            );
        }
        // An unrecognised state is not proof the bootloader is there either.
        assert!(!BootloaderState::from(9).in_bootloader());
    }

    #[test]
    fn version_formatting_names_the_responder_and_flags_a_dirty_tree() {
        assert_eq!(
            format_version(&version(false, false, false)),
            "app 0.1.0 git 7e21b0"
        );
        assert_eq!(
            format_version(&version(true, true, false)),
            "boot 0.1.0 git 7e21b0-dirty"
        );
        // A non-git build must not print a hash that looks real.
        assert_eq!(
            format_version(&version(false, false, true)),
            "app 0.1.0 git unknown"
        );
    }

    #[test]
    fn discovery_pairs_each_boards_state_with_its_version() {
        let boot = version(true, false, false);
        let app = version(false, true, false);
        let found = merge_discovery(
            &[
                (AppType::RudderController, BootloaderState::WaitingWithApp),
                (AppType::Dashboard, BootloaderState::AppRunning),
            ],
            &[(AppType::Dashboard, app), (AppType::RudderController, boot)],
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].app_type, Some(AppType::RudderController));
        assert_eq!(found[0].version, Some(boot));
        assert_eq!(found[1].app_type, Some(AppType::Dashboard));
        assert_eq!(found[1].version, Some(app));
    }

    /// A bootloader flashed before GetVersion existed answers the state
    /// broadcast and ignores the version one. It still has to appear.
    #[test]
    fn a_board_with_no_version_reply_is_still_listed() {
        let found = merge_discovery(
            &[(
                AppType::HeightSensorController,
                BootloaderState::WaitingWithApp,
            )],
            &[],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].state, BootloaderState::WaitingWithApp);
        assert_eq!(found[0].version, None);
    }

    /// The reverse — a dropped state frame — must not hide the board either.
    #[test]
    fn a_board_with_only_a_version_reply_is_still_listed() {
        let v = version(false, false, false);
        let found = merge_discovery(&[], &[(AppType::Dashboard, v)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].app_type, Some(AppType::Dashboard));
        assert_eq!(found[0].version, Some(v));
        assert!(!found[0].state.in_bootloader());
    }

    #[test]
    fn an_empty_bus_yields_no_boards() {
        assert!(merge_discovery(&[], &[]).is_empty());
    }
}
