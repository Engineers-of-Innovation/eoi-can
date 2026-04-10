use defmt::*;
use embassy_stm32::can::{BufferedCan, Frame, Id, StandardId};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_time::{Duration, Timer, with_timeout};
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

use crate::flash::FlashLayout;
use crate::header::{HeaderError, HeaderInfo, ValidationResult};

// CAN IDs for bootloader protocol
const CAN_ID_HOST_TO_DEVICE: StandardId = unsafe { StandardId::new_unchecked(0x030) };
const CAN_ID_DEVICE_TO_HOST: StandardId = unsafe { StandardId::new_unchecked(0x031) };
const CAN_ID_WRITE_DATA: StandardId = unsafe { StandardId::new_unchecked(0x032) };

/// Message types (byte 0 of CAN command/response frames)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Format)]
#[repr(u8)]
enum MsgType {
    GetState = 0x01,
    EraseApp = 0x02,
    WriteAck = 0x03,
    ValidateApp = 0x04,
    BootApp = 0x05,
    Reboot = 0x06,
    Error = 0xFF,
}

impl MsgType {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::GetState),
            0x02 => Some(Self::EraseApp),
            0x03 => Some(Self::WriteAck),
            0x04 => Some(Self::ValidateApp),
            0x05 => Some(Self::BootApp),
            0x06 => Some(Self::Reboot),
            0xFF => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Format)]
#[repr(u8)]
enum BootloaderState {
    WaitingWithoutApp = 0,
    WaitingWithApp = 1,
    FlashingApp = 2,
}

pub struct Bootloader<'a, M: RawMutex, T: NorFlash + ReadNorFlash> {
    state: BootloaderState,
    flash: FlashLayout<'a, M, T>,
    write_offset: usize,
    can: BufferedCan<'static, 8, 8>,
    watchdog: IndependentWatchdog<'static, embassy_stm32::peripherals::IWDG>,
}

impl<'a, M, T> Bootloader<'a, M, T>
where
    M: RawMutex,
    T: NorFlash + ReadNorFlash,
{
    pub fn init(
        flash: FlashLayout<'a, M, T>,
        can: BufferedCan<'static, 8, 8>,
        iwdg: embassy_stm32::Peri<'static, embassy_stm32::peripherals::IWDG>,
    ) -> Self {
        let state = if validate_app(&flash).is_ok() {
            info!("Valid application found");
            BootloaderState::WaitingWithApp
        } else {
            info!("No valid application found");
            BootloaderState::WaitingWithoutApp
        };

        // 4 second timeout — app must pet the watchdog within this window
        let watchdog = IndependentWatchdog::new(iwdg, 4_000_000);

        Self {
            state,
            flash,
            write_offset: 0,
            can,
            watchdog,
        }
    }

    pub async fn run(&mut self) -> ! {
        loop {
            // Send current state
            self.send_state().await;

            // Wait for a CAN frame with timeout
            let result = with_timeout(Duration::from_secs(2), self.can.read()).await;

            match result {
                Ok(Ok(envelope)) => {
                    let frame = &envelope.frame;
                    let data = frame.data();

                    if *frame.id() == Id::Standard(CAN_ID_WRITE_DATA) {
                        // Dedicated write data frames: all 8 bytes are payload
                        if let Err(e) = self.handle_write_data(data).await {
                            warn!("Write error: {}", e);
                            self.send_error(e).await;
                        }
                    } else if *frame.id() == Id::Standard(CAN_ID_HOST_TO_DEVICE) {
                        if data.is_empty() {
                            continue;
                        }
                        if let Err(e) = self.process_command(data[0]).await {
                            warn!("Command error: {}", e);
                            self.send_error(e).await;
                        }
                    }
                }
                Ok(Err(_bus_err)) => {
                    warn!("CAN bus error");
                }
                Err(_timeout) => {
                    // Auto-boot if valid app present and no commands received
                    if self.state == BootloaderState::WaitingWithApp {
                        info!("Timeout, auto-booting application");
                        self.boot_app().await;
                    }
                    // Check if flashing completed
                    if self.state == BootloaderState::FlashingApp
                        && validate_app(&self.flash).is_ok()
                    {
                        self.state = BootloaderState::WaitingWithApp;
                    }
                }
            }
        }
    }

    async fn process_command(&mut self, cmd: u8) -> Result<(), u8> {
        let Some(msg) = MsgType::from_u8(cmd) else {
            return Err(0x05);
        };
        match msg {
            MsgType::GetState => {
                self.send_state().await;
            }
            MsgType::EraseApp => {
                info!("Erasing application");
                self.flash.erase_header_and_app().await.map_err(|_| 0x01)?;
                self.write_offset = 0;
                self.state = BootloaderState::WaitingWithoutApp;
                self.send_response(&[MsgType::EraseApp as u8]).await;
                info!("Erase complete");
            }
            MsgType::ValidateApp => {
                let result = match validate_app(&self.flash) {
                    Ok(_) => {
                        self.state = BootloaderState::WaitingWithApp;
                        ValidationResult::Valid
                    }
                    Err(HeaderError::BadMagic) => ValidationResult::BadMagic,
                    Err(
                        HeaderError::ZeroLength
                        | HeaderError::AppTooLarge
                        | HeaderError::BadVersion,
                    ) => ValidationResult::BadLength,
                    Err(HeaderError::BadAppCrc) => ValidationResult::BadCrc,
                };
                self.send_response(&[MsgType::ValidateApp as u8, result as u8])
                    .await;
            }
            MsgType::BootApp => {
                if validate_app(&self.flash).is_err() {
                    return Err(0x04);
                }
                self.send_response(&[MsgType::BootApp as u8]).await;
                Timer::after(Duration::from_millis(500)).await;
                self.boot_app().await;
            }
            MsgType::Reboot => {
                self.send_response(&[MsgType::Reboot as u8]).await;
                Timer::after(Duration::from_millis(500)).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            _ => {
                return Err(0x05);
            }
        }
        Ok(())
    }

    /// Handle a write data frame received on the dedicated CAN_ID_WRITE_DATA address.
    /// All 8 bytes of the frame are raw data, written directly to flash (8-byte aligned).
    async fn handle_write_data(&mut self, data: &[u8]) -> Result<(), u8> {
        if data.len() != 8 {
            return Err(0x02);
        }
        let address = self.flash.header_address() + self.write_offset;
        self.flash.write_bytes(address, data).map_err(|_| 0x03)?;
        self.write_offset += 8;
        self.state = BootloaderState::FlashingApp;

        let offset_bytes = (self.write_offset as u32).to_le_bytes();
        self.send_response(&[
            MsgType::WriteAck as u8,
            offset_bytes[0],
            offset_bytes[1],
            offset_bytes[2],
            offset_bytes[3],
        ])
        .await;
        Ok(())
    }

    async fn send_state(&mut self) {
        self.send_response(&[MsgType::GetState as u8, self.state as u8])
            .await;
    }

    async fn send_error(&mut self, code: u8) {
        self.send_response(&[MsgType::Error as u8, code]).await;
    }

    async fn send_response(&mut self, data: &[u8]) {
        if let Ok(frame) = Frame::new_data(CAN_ID_DEVICE_TO_HOST, data) {
            self.can.write(&frame).await;
        }
    }

    async fn boot_app(&mut self) -> ! {
        let boot_address = self.flash.app_address();
        info!("Booting application at 0x{:08X}", boot_address);

        // Start the watchdog before jumping — if the app crashes, MCU resets back to bootloader
        self.watchdog.unleash();

        unsafe {
            cortex_m::interrupt::disable();

            let nvic = &*cortex_m::peripheral::NVIC::PTR;
            // Disable all configurable interrupts
            for clear_enable in &nvic.icer {
                clear_enable.write(u32::MAX);
            }
            // Clear all interrupt-pending bits
            for clear_pending in &nvic.icpr {
                clear_pending.write(u32::MAX);
            }
            // Reset all interrupt priorities
            for priority in &nvic.ipr {
                priority.write(0);
            }

            // Re-enable interrupts globally to match boot-up environment.
            cortex_m::interrupt::enable();

            let mut p = cortex_m::Peripherals::steal();
            p.SCB.invalidate_icache();
            p.SCB.vtor.write(boot_address as u32);

            cortex_m::asm::bootload(boot_address as *const u32)
        }
    }
}

fn validate_app<M: RawMutex, T: NorFlash + ReadNorFlash>(
    flash: &FlashLayout<'_, M, T>,
) -> Result<HeaderInfo, HeaderError> {
    let header_bytes = flash.read_header().map_err(|_| HeaderError::BadMagic)?;

    // Log the first 16 bytes of the header for diagnostics
    info!("Header raw: {:02X}", &header_bytes[..16]);

    let header = HeaderInfo::from_bytes(&header_bytes)?;
    info!(
        "Header parsed: app_len={} app_crc=0x{:08X}",
        header.app_len, header.app_crc
    );

    let max_app_len = flash.max_app_len();
    let mut read_offset: usize = 0;
    header.validate(max_app_len, &mut |buf| {
        let n = flash.read_app(read_offset, buf).map_err(|_| ())?;
        read_offset += n;
        Ok(n)
    })?;

    Ok(header)
}
