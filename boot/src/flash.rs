use core::cell::RefCell;
use core::ffi::c_void;

use defmt::*;
use embassy_futures::yield_now;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

use eoi_boot_api::header::HEADER_PARTITION_SIZE;

pub struct FlashLayout<'a, M: RawMutex, T: NorFlash + ReadNorFlash> {
    flash: &'a Mutex<M, RefCell<T>>,
    flash_base_address: usize,
    header_offset: usize,
    app_offset: usize,
    app_len: usize,
}

impl<'a, M: RawMutex, T: NorFlash + ReadNorFlash> FlashLayout<'a, M, T> {
    pub fn new(flash: &'a Mutex<M, RefCell<T>>) -> Self {
        unsafe extern "C" {
            static __flash_start: c_void;
            static __header_start: c_void;
            static __header_end: c_void;
            static __app_start: c_void;
            static __app_end: c_void;
        }

        let flash_base_address = &raw const __flash_start as usize;
        let header_start = &raw const __header_start as usize;
        let header_end = &raw const __header_end as usize;
        let app_start = &raw const __app_start as usize;
        let app_end = &raw const __app_end as usize;

        let header_offset = header_start - flash_base_address;
        let header_len = header_end - header_start;
        let app_offset = app_start - flash_base_address;
        let app_len = app_end - app_start;

        info!(
            "Flash layout: base=0x{:08X} header=0x{:08X}({}B) app=0x{:08X}({}B)",
            flash_base_address, header_start, header_len, app_start, app_len
        );

        core::assert_eq!(
            header_len, HEADER_PARTITION_SIZE,
            "Header partition size mismatch"
        );

        Self {
            flash,
            flash_base_address,
            header_offset,
            app_offset,
            app_len,
        }
    }

    pub fn app_address(&self) -> usize {
        self.flash_base_address + self.app_offset
    }

    pub fn header_address(&self) -> usize {
        self.flash_base_address + self.header_offset
    }

    pub fn max_app_len(&self) -> u32 {
        self.app_len as u32
    }

    pub fn read_header(&self) -> Result<[u8; HEADER_PARTITION_SIZE], ()> {
        let mut buf = [0xFF; HEADER_PARTITION_SIZE];
        self.flash.lock(|flash| {
            flash
                .borrow_mut()
                .read(self.header_offset as u32, &mut buf)
                .map_err(|_| ())
        })?;
        Ok(buf)
    }

    /// Read `len` bytes from the app partition starting at offset 0.
    /// Returns number of bytes actually read into `buf`.
    pub fn read_app(&self, offset: usize, buf: &mut [u8]) -> Result<usize, ()> {
        let available = self.app_len.saturating_sub(offset);
        let to_read = buf.len().min(available);
        if to_read == 0 {
            return Ok(0);
        }
        self.flash.lock(|flash| {
            flash
                .borrow_mut()
                .read((self.app_offset + offset) as u32, &mut buf[..to_read])
                .map_err(|_| ())
        })?;
        Ok(to_read)
    }

    /// Write bytes to flash. `address` is an absolute address (base + offset).
    pub fn write_bytes(&self, address: usize, data: &[u8]) -> Result<(), ()> {
        let offset = address
            .checked_sub(self.flash_base_address)
            .ok_or(())?;
        // Must be within header or app partition
        if offset < self.header_offset || offset + data.len() > self.app_offset + self.app_len {
            return Err(());
        }
        self.flash.lock(|flash| {
            flash
                .borrow_mut()
                .write(offset as u32, data)
                .map_err(|_| ())
        })
    }

    /// Erase the header and app partitions page by page.
    pub async fn erase_header_and_app(&self) -> Result<(), ()> {
        let total = HEADER_PARTITION_SIZE + self.app_len;
        let erase_size = T::ERASE_SIZE;
        for offset in (0..total).step_by(erase_size) {
            let from = (self.header_offset + offset) as u32;
            let to = from + erase_size as u32;
            self.flash.lock(|flash| {
                flash.borrow_mut().erase(from, to).map_err(|_| ())
            })?;
            yield_now().await;
        }
        Ok(())
    }
}
