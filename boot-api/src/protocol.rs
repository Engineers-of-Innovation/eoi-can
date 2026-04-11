/// CAN ID for host-to-device commands.
pub const CAN_ID_HOST_TO_DEVICE: u16 = 0x030;
/// CAN ID for device-to-host responses.
pub const CAN_ID_DEVICE_TO_HOST: u16 = 0x031;
/// CAN ID for bulk write data frames.
pub const CAN_ID_WRITE_DATA: u16 = 0x032;

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
