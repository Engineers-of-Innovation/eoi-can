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
    pub const ERROR: u8 = 0xFF;
}
