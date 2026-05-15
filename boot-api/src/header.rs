use crc::{CRC_32_ISCSI, Crc};

pub const HEADER_MAGIC: [u8; 4] = [0xB0, 0x07, 0xCA, 0xFE];
pub const HEADER_VERSION: u8 = 1;
pub const HEADER_PARTITION_SIZE: usize = 2048;

const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// Application type baked into the firmware header. The bootloader is
/// compiled for one specific variant and refuses to boot an image whose
/// type doesn't match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum AppType {
    RudderController = 0x01,
    HeightSensorController = 0x02,
}

impl AppType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::RudderController),
            0x02 => Some(Self::HeightSensorController),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct HeaderInfo {
    pub app_len: u32,
    pub app_crc: u32,
    pub app_type: AppType,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HeaderError {
    BadMagic,
    BadVersion,
    ZeroLength,
    AppTooLarge,
    BadAppCrc,
    BadAppType,
    WrongAppType,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum ValidationResult {
    Valid = 0,
    BadMagic = 1,
    BadLength = 2,
    BadCrc = 3,
    WrongAppType = 4,
}

impl HeaderInfo {
    /// Parse header from the raw 2048-byte partition.
    pub fn from_bytes(data: &[u8; HEADER_PARTITION_SIZE]) -> Result<Self, HeaderError> {
        if data[0..4] != HEADER_MAGIC {
            return Err(HeaderError::BadMagic);
        }
        if data[4] != HEADER_VERSION {
            return Err(HeaderError::BadVersion);
        }
        let app_len = u32::from_le_bytes([data[5], data[6], data[7], data[8]]);
        let app_crc = u32::from_le_bytes([data[9], data[10], data[11], data[12]]);
        if app_len == 0 {
            return Err(HeaderError::ZeroLength);
        }
        let app_type = AppType::from_u8(data[13]).ok_or(HeaderError::BadAppType)?;
        Ok(Self {
            app_len,
            app_crc,
            app_type,
        })
    }

    /// Validate that app_len fits in the partition and CRC matches.
    pub fn validate(
        &self,
        max_app_len: u32,
        app_data_reader: &mut dyn FnMut(&mut [u8]) -> Result<usize, ()>,
    ) -> Result<(), HeaderError> {
        if self.app_len > max_app_len {
            return Err(HeaderError::AppTooLarge);
        }

        let mut digest = CRC.digest();
        let mut remaining = self.app_len as usize;
        let mut buf = [0u8; 256];
        while remaining > 0 {
            let to_read = remaining.min(buf.len());
            let n = app_data_reader(&mut buf[..to_read]).map_err(|_| HeaderError::BadAppCrc)?;
            if n == 0 {
                break;
            }
            digest.update(&buf[..n]);
            remaining -= n;
        }

        if digest.finalize() != self.app_crc {
            return Err(HeaderError::BadAppCrc);
        }
        Ok(())
    }
}

/// Build a 2048-byte header for the given app.
pub fn build_header(app_len: u32, app_crc: u32, app_type: AppType) -> [u8; HEADER_PARTITION_SIZE] {
    let mut header = [0xFF_u8; HEADER_PARTITION_SIZE];
    header[0..4].copy_from_slice(&HEADER_MAGIC);
    header[4] = HEADER_VERSION;
    header[5..9].copy_from_slice(&app_len.to_le_bytes());
    header[9..13].copy_from_slice(&app_crc.to_le_bytes());
    header[13] = app_type as u8;
    header
}

/// Compute CRC32-ISCSI over a byte slice.
pub fn compute_crc(data: &[u8]) -> u32 {
    let mut d = CRC.digest();
    d.update(data);
    d.finalize()
}
