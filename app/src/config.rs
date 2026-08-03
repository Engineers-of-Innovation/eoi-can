//! Persistent configuration in emulated EEPROM.
//!
//! The STM32L471 has no data EEPROM, so a dedicated 4 K block behind the
//! application partition (0x080FF000..0x080FFFFF, the last two pages of flash
//! bank 2) holds the configuration instead. The block sits outside the app
//! partition, so a firmware update over CAN leaves it untouched.
//!
//! The block is an append-only log of fixed-size slots. A write lands in the
//! first erased slot; on load, the newest slot that passes magic/version/CRC
//! wins. When the log is full the block is erased and writing restarts at slot
//! 0, giving 256 writes per erase cycle — so the 10 k cycle flash endurance is
//! never the practical limit.
//!
//! The block lives in bank 2 while code executes from bank 1, so erasing and
//! programming do not stall the CPU.

use defmt::*;
use embassy_stm32::Peri;
use embassy_stm32::flash::{
    Blocking, Error as FlashError, FLASH_SIZE, Flash, MAX_ERASE_SIZE, WRITE_SIZE,
};
use embassy_stm32::pac;
use embassy_stm32::peripherals::FLASH;
use eoi_boot_api::header::compute_crc;

/// Offset of the config block from the start of flash (absolute 0x080FF000).
/// Must match the CONFIG region in `linker/app.x` and `linker/app-dev.x`, and
/// `__app_end` in `linker/boot.x`.
const BLOCK_OFFSET: u32 = 0x000F_F000;
const BLOCK_SIZE: u32 = 4 * 1024;

/// One record. 16 bytes, a multiple of the 8-byte flash write granularity.
const SLOT_SIZE: usize = 16;
const SLOTS: usize = BLOCK_SIZE as usize / SLOT_SIZE;

const MAGIC: u16 = 0xCA1B;
const VERSION: u8 = 1;

// The block must be the last thing in flash, page-aligned at both ends, and
// divide evenly into writable slots. `core::assert!` because `defmt::*` brings
// its own non-const `assert!` into scope.
const _: () = core::assert!(BLOCK_OFFSET as usize + BLOCK_SIZE as usize == FLASH_SIZE);
const _: () = core::assert!((BLOCK_OFFSET as usize).is_multiple_of(MAX_ERASE_SIZE));
const _: () = core::assert!((BLOCK_SIZE as usize).is_multiple_of(MAX_ERASE_SIZE));
const _: () = core::assert!(SLOT_SIZE.is_multiple_of(WRITE_SIZE));

/// Bits in [`SteeringCal::captured`] marking which endpoints have been taken.
pub const CAPTURED_LEFT: u8 = 1 << 0;
pub const CAPTURED_CENTER: u8 = 1 << 1;
pub const CAPTURED_RIGHT: u8 = 1 << 2;
pub const CAPTURED_ALL: u8 = CAPTURED_LEFT | CAPTURED_CENTER | CAPTURED_RIGHT;

/// Raw ADC codes captured at the three steering reference positions.
#[derive(Clone, Copy, Default, PartialEq, Eq, Format)]
pub struct SteeringCal {
    /// Which endpoints have been captured (`CAPTURED_*` bits).
    pub captured: u8,
    pub left: u16,
    pub center: u16,
    pub right: u16,
}

impl SteeringCal {
    fn encode(&self) -> [u8; SLOT_SIZE] {
        // Bytes 10..12 are reserved for future fields and must stay zero so
        // the CRC over an unchanged record is reproducible.
        let mut slot = [0u8; SLOT_SIZE];
        slot[0..2].copy_from_slice(&MAGIC.to_le_bytes());
        slot[2] = VERSION;
        slot[3] = self.captured;
        slot[4..6].copy_from_slice(&self.left.to_le_bytes());
        slot[6..8].copy_from_slice(&self.center.to_le_bytes());
        slot[8..10].copy_from_slice(&self.right.to_le_bytes());
        let crc = compute_crc(&slot[0..12]);
        slot[12..16].copy_from_slice(&crc.to_le_bytes());
        slot
    }

    fn decode(slot: &[u8; SLOT_SIZE]) -> Option<Self> {
        if u16::from_le_bytes([slot[0], slot[1]]) != MAGIC || slot[2] != VERSION {
            return None;
        }
        let crc = u32::from_le_bytes([slot[12], slot[13], slot[14], slot[15]]);
        if compute_crc(&slot[0..12]) != crc {
            return None;
        }
        Some(Self {
            captured: slot[3],
            left: u16::from_le_bytes([slot[4], slot[5]]),
            center: u16::from_le_bytes([slot[6], slot[7]]),
            right: u16::from_le_bytes([slot[8], slot[9]]),
        })
    }
}

pub struct ConfigStore {
    flash: Flash<'static, Blocking>,
    /// Index of the first erased slot, or `SLOTS` when the block is full.
    next_slot: usize,
}

impl ConfigStore {
    pub fn new(flash: Peri<'static, FLASH>) -> Self {
        Self {
            flash: Flash::new_blocking(flash),
            next_slot: 0,
        }
    }

    /// Scan the block and return the newest record that passes magic/version/CRC,
    /// leaving the write cursor past the last slot in use.
    ///
    /// `None` means nothing has ever been stored, or every record is corrupt.
    pub fn load(&mut self) -> Option<SteeringCal> {
        let mut newest = None;
        self.next_slot = 0;
        for index in 0..SLOTS {
            let mut slot = [0u8; SLOT_SIZE];
            let in_use = match self.flash.blocking_read(slot_offset(index), &mut slot) {
                Ok(()) => !slot.iter().all(|&b| b == 0xFF),
                Err(e) => {
                    warn!("Config slot {} read error: {:?}", index, e);
                    // Can't tell whether it is erased, so assume it is taken
                    // rather than risk programming over it.
                    true
                }
            };
            if !in_use {
                continue;
            }
            // A torn write leaves a slot that is neither erased nor decodable;
            // it still consumes the slot, so keep the cursor moving past it.
            self.next_slot = index + 1;
            if let Some(cal) = SteeringCal::decode(&slot) {
                newest = Some(cal);
            }
        }
        newest
    }

    /// Append a record, erasing the block first when the slot log is full.
    pub fn store(&mut self, cal: &SteeringCal) -> Result<(), FlashError> {
        if self.next_slot >= SLOTS {
            info!("Config block full, erasing");
            self.flash
                .blocking_erase(BLOCK_OFFSET, BLOCK_OFFSET + BLOCK_SIZE)?;
            flush_caches();
            self.next_slot = 0;
        }

        let offset = slot_offset(self.next_slot);
        // Burn the slot either way: a failed program may have left it partly
        // written, and programming an already-written slot always fails.
        self.next_slot += 1;
        let result = self.flash.blocking_write(offset, &cal.encode());
        flush_caches();
        result
    }
}

fn slot_offset(index: usize) -> u32 {
    BLOCK_OFFSET + (index * SLOT_SIZE) as u32
}

/// Reset the flash instruction and data caches.
///
/// Erasing or programming leaves cached lines stale and embassy's flash driver
/// does not reset the caches itself, so a later read of the config block could
/// otherwise still see pre-write data.
fn flush_caches() {
    cortex_m::interrupt::free(|_| {
        let acr = pac::FLASH.acr().read();
        if acr.icen() {
            pac::FLASH.acr().modify(|w| w.set_icen(false));
            pac::FLASH.acr().modify(|w| w.set_icrst(true));
            pac::FLASH.acr().modify(|w| w.set_icrst(false));
            pac::FLASH.acr().modify(|w| w.set_icen(true));
        }
        if acr.dcen() {
            pac::FLASH.acr().modify(|w| w.set_dcen(false));
            pac::FLASH.acr().modify(|w| w.set_dcrst(true));
            pac::FLASH.acr().modify(|w| w.set_dcrst(false));
            pac::FLASH.acr().modify(|w| w.set_dcen(true));
        }
    });
}
