use crate::header::AppType;

/// CAN ID for the host's discovery broadcast. Every bootloader listens on this
/// ID and answers on its own [`BoardAddress::resp`], so no two nodes ever
/// transmit the same ID.
pub const CAN_ID_DISCOVERY: u16 = 0x030;

/// First ID of the per-board block, immediately after the discovery ID.
const ADDRESS_BASE: u16 = 0x031;
/// IDs owned by each board: cmd, resp, data.
const ADDRESS_STRIDE: u16 = 3;
/// Last ID the bootloader block owns. A sixth [`AppType`] would overflow it and
/// has to extend the allocation into 0x040+ (free, but a protocol doc change).
pub const ADDRESS_LAST: u16 = 0x03F;

/// The three CAN IDs a bootloader of a given app type owns.
///
/// Each board type gets its own block, so a command can only ever reach the
/// board it is addressed to — the bootloader's hardware filter rejects the
/// other blocks outright, which is what keeps an `ERASE_APP` aimed at one board
/// from wiping the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BoardAddress {
    /// Host -> board commands, byte 0 = message type.
    pub cmd: u16,
    /// Board -> host responses. This board is the only transmitter.
    pub resp: u16,
    /// Host -> board bulk write data, all 8 bytes payload.
    pub data: u16,
}

/// The CAN IDs owned by the bootloader of `app_type`.
pub const fn board_address(app_type: AppType) -> BoardAddress {
    let base = ADDRESS_BASE + (app_type as u16 - 1) * ADDRESS_STRIDE;
    assert!(
        base + ADDRESS_STRIDE - 1 <= ADDRESS_LAST,
        "app type overflows the 0x030-0x03F bootloader ID block"
    );
    BoardAddress {
        cmd: base,
        resp: base + 1,
        data: base + 2,
    }
}

/// The app type that owns `resp_id`, if any. Used by the host to attribute
/// discovery replies to a board.
pub fn app_type_from_resp_id(resp_id: u16) -> Option<AppType> {
    if !(ADDRESS_BASE..=ADDRESS_LAST).contains(&resp_id) {
        return None;
    }
    // Response IDs sit one past each block's base; anything else is a host-side
    // ID (cmd/data) or the discovery ID, none of which identify a board.
    let offset = resp_id - ADDRESS_BASE;
    if offset % ADDRESS_STRIDE != 1 {
        return None;
    }
    AppType::from_u8((offset / ADDRESS_STRIDE + 1) as u8)
}

/// Message types used in byte 0 of CAN command/response frames.
pub mod msg {
    pub const GET_STATE: u8 = 0x01;
    pub const ERASE_APP: u8 = 0x02;
    pub const WRITE_ACK: u8 = 0x03;
    pub const VALIDATE_APP: u8 = 0x04;
    pub const BOOT_APP: u8 = 0x05;
    pub const REBOOT: u8 = 0x06;
    pub const GET_VERSION: u8 = 0x07;
    pub const ERROR: u8 = 0xFF;
}

/// Payload byte 1 of an `ERROR` response.
pub mod err {
    pub const ERASE_FAILED: u8 = 0x01;
    pub const BAD_WRITE_LEN: u8 = 0x02;
    pub const WRITE_FAILED: u8 = 0x03;
    pub const NO_VALID_APP: u8 = 0x04;
    /// The responder does not implement this command. Also what a bootloader
    /// built before `GET_VERSION` answers when asked for a version.
    pub const UNKNOWN_COMMAND: u8 = 0x05;
    /// The application is running; this command needs the bootloader.
    pub const APP_RUNNING: u8 = 0x06;
}

/// State byte a running application reports in a `GET_STATE` response.
///
/// The bootloader owns 0..=2 (`WaitingWithoutApp`, `WaitingWithApp`,
/// `FlashingApp`); this value can only come from an application, which is how
/// the host tells the two apart on a single response ID.
pub const STATE_APP_RUNNING: u8 = 0x03;

/// Build identity of whatever is running on a board, as carried by one
/// `GET_VERSION` response frame.
///
/// Bootloader and application are flashed independently and can be built from
/// different commits, so each reports its own — [`Self::bootloader`] says which
/// one answered.
///
/// Wire format, exactly 8 bytes:
/// `[GET_VERSION, major, minor, patch, git0, git1, git2, flags]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct VersionInfo {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    /// First three bytes of the commit hash — six hex chars, plenty to pin a
    /// commit in this repo and all that is left in the frame.
    pub git: [u8; 3],
    /// Working tree had uncommitted changes at build time.
    pub dirty: bool,
    /// The bootloader answered, not the application.
    pub bootloader: bool,
    /// Not built from a git checkout; [`Self::git`] is meaningless.
    pub git_unknown: bool,
}

/// Bit positions in the flags byte (payload byte 7).
const FLAG_DIRTY: u8 = 1 << 0;
const FLAG_BOOTLOADER: u8 = 1 << 1;
const FLAG_GIT_UNKNOWN: u8 = 1 << 2;

impl VersionInfo {
    pub const fn encode(&self) -> [u8; 8] {
        let mut flags = 0;
        if self.dirty {
            flags |= FLAG_DIRTY;
        }
        if self.bootloader {
            flags |= FLAG_BOOTLOADER;
        }
        if self.git_unknown {
            flags |= FLAG_GIT_UNKNOWN;
        }
        [
            msg::GET_VERSION,
            self.major,
            self.minor,
            self.patch,
            self.git[0],
            self.git[1],
            self.git[2],
            flags,
        ]
    }

    /// Decode a `GET_VERSION` response payload, including its leading type byte.
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 8 || data[0] != msg::GET_VERSION {
            return None;
        }
        Some(Self {
            major: data[1],
            minor: data[2],
            patch: data[3],
            git: [data[4], data[5], data[6]],
            dirty: data[7] & FLAG_DIRTY != 0,
            bootloader: data[7] & FLAG_BOOTLOADER != 0,
            git_unknown: data[7] & FLAG_GIT_UNKNOWN != 0,
        })
    }

    /// Build the constant a firmware reports, from the strings the `built`
    /// crate generates. `const` so the whole thing folds away at compile time
    /// and no parsing code reaches the device.
    pub const fn from_built(
        major: &str,
        minor: &str,
        patch: &str,
        hash: Option<&str>,
        dirty: Option<bool>,
        bootloader: bool,
    ) -> Self {
        let (git, git_unknown) = match hash {
            Some(h) => (parse_hash_prefix(h), false),
            None => ([0, 0, 0], true),
        };
        Self {
            major: parse_u8(major),
            minor: parse_u8(minor),
            patch: parse_u8(patch),
            git,
            dirty: match dirty {
                Some(d) => d,
                // Unknown dirtiness is reported as dirty: an image that might
                // not match its commit must not claim it does.
                None => true,
            },
            bootloader,
            git_unknown,
        }
    }
}

/// What a running application should do with a received frame.
///
/// The decision is kept separate from the CAN driver so it can be tested on the
/// host: getting it wrong either bricks OTA updates (no reboot) or resets the
/// wrong board (reboot on someone else's command ID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AppAction {
    /// Transmit `data[..len]` on `id`.
    Reply { id: u16, data: [u8; 8], len: usize },
    /// System reset, so the bootloader takes over. No response is sent — the
    /// reset lands long before a queued frame could leave the TX mailbox.
    Reboot,
}

impl AppAction {
    const fn reply(id: u16, data: [u8; 8], len: usize) -> Self {
        Self::Reply { id, data, len }
    }

    /// The bytes to transmit, for a [`Self::Reply`].
    pub fn payload(&self) -> Option<&[u8]> {
        match self {
            Self::Reply { data, len, .. } => Some(&data[..*len]),
            Self::Reboot => None,
        }
    }
}

/// Decide how a running application answers one received frame.
///
/// `frame_id` is the standard CAN ID it arrived on and `data` its payload.
/// Applications run an accept-all hardware filter (the dashboard needs the
/// whole bus), so scoping happens here: only this board's command ID and the
/// discovery broadcast are acted on, and only the command ID can trigger a
/// reset.
///
/// `GET_STATE` and `GET_VERSION` are answered in both places, which is what
/// makes a running board show up in `scan` and answer `state` / `version`.
/// Everything else needs the bootloader and is rejected with
/// [`err::APP_RUNNING`] — but only when addressed, since an unrecognised
/// broadcast is not ours to reject.
pub fn app_action(
    frame_id: u16,
    data: &[u8],
    app_type: AppType,
    version: &VersionInfo,
) -> Option<AppAction> {
    let addr = board_address(app_type);
    let addressed = frame_id == addr.cmd;
    if !addressed && frame_id != CAN_ID_DISCOVERY {
        return None;
    }
    let &cmd = data.first()?;

    match cmd {
        msg::GET_STATE => Some(AppAction::reply(
            addr.resp,
            [
                msg::GET_STATE,
                STATE_APP_RUNNING,
                app_type as u8,
                0,
                0,
                0,
                0,
                0,
            ],
            3,
        )),
        msg::GET_VERSION => Some(AppAction::reply(addr.resp, version.encode(), 8)),
        msg::REBOOT if addressed => Some(AppAction::Reboot),
        _ if addressed => Some(AppAction::reply(
            addr.resp,
            [msg::ERROR, err::APP_RUNNING, 0, 0, 0, 0, 0, 0],
            2,
        )),
        _ => None,
    }
}

/// Decimal parse for `const` contexts. Saturates rather than panicking — a
/// version component past 255 is a packaging mistake, not worth failing the
/// build over, and it still shows up as an obviously wrong number.
const fn parse_u8(s: &str) -> u8 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut v: u32 = 0;
    while i < b.len() {
        if b[i] < b'0' || b[i] > b'9' {
            break;
        }
        v = v * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    if v > 255 { 255 } else { v as u8 }
}

/// First three bytes of a hex commit hash. A short string yields zeros for the
/// bytes it cannot fill; the host prints whatever it is given.
const fn parse_hash_prefix(s: &str) -> [u8; 3] {
    let b = s.as_bytes();
    let mut out = [0u8; 3];
    let mut i = 0;
    while i < 3 {
        if 2 * i + 1 >= b.len() {
            break;
        }
        out[i] = hex_nibble(b[2 * i]) << 4 | hex_nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}

const fn hex_nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}
