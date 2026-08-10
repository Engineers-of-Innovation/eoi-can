use defmt::*;
use embassy_stm32::can::{BufferedCan, Frame, Id, StandardId};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_time::{Duration, Instant, Timer, with_deadline};
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

use crate::flash::FlashLayout;
use crate::{BOARD_ADDRESS, EXPECTED_APP_TYPE, std_id};
use eoi_boot_api::header::{HeaderError, HeaderInfo, ValidationResult};
use eoi_boot_api::protocol;

/// Broadcast the host uses to enumerate the bus. Answered on `CAN_ID_RESPONSE`,
/// never on a shared ID, so no two nodes ever transmit the same identifier.
const CAN_ID_DISCOVERY: StandardId = std_id(protocol::CAN_ID_DISCOVERY);
const CAN_ID_COMMAND: StandardId = std_id(BOARD_ADDRESS.cmd);
const CAN_ID_RESPONSE: StandardId = std_id(BOARD_ADDRESS.resp);
const CAN_ID_WRITE_DATA: StandardId = std_id(BOARD_ADDRESS.data);

/// How long to wait for the host before auto-booting a valid application.
const BOOT_WINDOW: Duration = Duration::from_secs(2);

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
        // Absolute deadline, not a per-frame timeout: a frame must not be able to
        // postpone auto-boot indefinitely. Only host traffic addressed to this
        // board extends it — see the `CAN_ID_COMMAND`/`CAN_ID_WRITE_DATA` arms.
        let mut boot_deadline = Instant::now() + BOOT_WINDOW;

        // Announce ourselves once so a host that is already listening sees us
        // without having to poll.
        self.send_state().await;

        loop {
            match with_deadline(boot_deadline, self.can.read()).await {
                Ok(Ok(envelope)) => {
                    let frame = &envelope.frame;
                    let data = frame.data();

                    if *frame.id() == Id::Standard(CAN_ID_WRITE_DATA) {
                        boot_deadline = Instant::now() + BOOT_WINDOW;
                        // Dedicated write data frames: all 8 bytes are payload
                        if let Err(e) = self.handle_write_data(data).await {
                            warn!("Write error: {}", e);
                            self.send_error(e).await;
                        }
                    } else if *frame.id() == Id::Standard(CAN_ID_COMMAND) {
                        boot_deadline = Instant::now() + BOOT_WINDOW;
                        if data.is_empty() {
                            continue;
                        }
                        if let Err(e) = self.process_command(data[0]).await {
                            warn!("Command error: {}", e);
                            self.send_error(e).await;
                        }
                    } else if *frame.id() == Id::Standard(CAN_ID_DISCOVERY) {
                        // Answer the broadcast but deliberately do NOT extend the
                        // deadline: a host polling `scan` in a loop must not pin
                        // every board in the bootloader.
                        match data.first() {
                            Some(&protocol::msg::GET_STATE) => self.send_state().await,
                            Some(&protocol::msg::GET_VERSION) => self.send_version().await,
                            // Never error on the broadcast — it is not ours alone.
                            _ => {}
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
                    // Check if flashing completed. Deliberately after the auto-boot
                    // check, so a just-written app needs one further quiet window
                    // before it can auto-boot — that leaves the host room to send
                    // VALIDATE_APP / BOOT_APP without racing the jump.
                    if self.state == BootloaderState::FlashingApp
                        && validate_app(&self.flash).is_ok()
                    {
                        self.state = BootloaderState::WaitingWithApp;
                    }
                    // Nothing booted: keep waiting, and re-announce so a host that
                    // arrives later still finds us.
                    boot_deadline = Instant::now() + BOOT_WINDOW;
                    self.send_state().await;
                }
            }
        }
    }

    async fn process_command(&mut self, cmd: u8) -> Result<(), u8> {
        use protocol::{err, msg};
        match cmd {
            msg::GET_STATE => {
                self.send_state().await;
            }
            msg::GET_VERSION => {
                self.send_version().await;
            }
            msg::ERASE_APP => {
                info!("Erasing application");
                self.flash
                    .erase_header_and_app()
                    .await
                    .map_err(|_| err::ERASE_FAILED)?;
                self.write_offset = 0;
                self.state = BootloaderState::WaitingWithoutApp;
                self.send_response(&[msg::ERASE_APP]).await;
                info!("Erase complete");
            }
            msg::VALIDATE_APP => {
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
                    Err(HeaderError::BadAppType | HeaderError::WrongAppType) => {
                        ValidationResult::WrongAppType
                    }
                };
                self.send_response(&[msg::VALIDATE_APP, result as u8]).await;
            }
            msg::BOOT_APP => {
                if validate_app(&self.flash).is_err() {
                    return Err(err::NO_VALID_APP);
                }
                self.send_response(&[msg::BOOT_APP]).await;
                Timer::after(Duration::from_millis(500)).await;
                self.boot_app().await;
            }
            msg::REBOOT => {
                self.send_response(&[msg::REBOOT]).await;
                Timer::after(Duration::from_millis(500)).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            _ => {
                return Err(err::UNKNOWN_COMMAND);
            }
        }
        Ok(())
    }

    /// Handle a write data frame received on the dedicated CAN_ID_WRITE_DATA address.
    /// All 8 bytes of the frame are raw data, written directly to flash (8-byte aligned).
    async fn handle_write_data(&mut self, data: &[u8]) -> Result<(), u8> {
        if data.len() != 8 {
            return Err(protocol::err::BAD_WRITE_LEN);
        }
        let address = self.flash.header_address() + self.write_offset;
        self.flash
            .write_bytes(address, data)
            .map_err(|_| protocol::err::WRITE_FAILED)?;
        self.write_offset += 8;
        self.state = BootloaderState::FlashingApp;

        let offset_bytes = (self.write_offset as u32).to_le_bytes();
        self.send_response(&[
            protocol::msg::WRITE_ACK,
            offset_bytes[0],
            offset_bytes[1],
            offset_bytes[2],
            offset_bytes[3],
        ])
        .await;
        Ok(())
    }

    /// Report state, including this board's app type. The response ID already
    /// implies the app type; the byte lets the host cross-check it.
    async fn send_state(&mut self) {
        self.send_response(&[
            protocol::msg::GET_STATE,
            self.state as u8,
            EXPECTED_APP_TYPE as u8,
        ])
        .await;
    }

    /// Report this bootloader's own build identity — not the application's,
    /// which it has no way of knowing: the firmware header carries no commit.
    async fn send_version(&mut self) {
        self.send_response(&crate::build_info::VERSION.encode())
            .await;
    }

    async fn send_error(&mut self, code: u8) {
        self.send_response(&[protocol::msg::ERROR, code]).await;
    }

    async fn send_response(&mut self, data: &[u8]) {
        if let Ok(frame) = Frame::new_data(CAN_ID_RESPONSE, data) {
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
        "Header parsed: app_len={} app_crc=0x{:08X} app_type={}",
        header.app_len, header.app_crc, header.app_type
    );

    if header.app_type != EXPECTED_APP_TYPE {
        warn!(
            "App type mismatch: image is {}, board expects {}",
            header.app_type, EXPECTED_APP_TYPE
        );
        return Err(HeaderError::WrongAppType);
    }

    let max_app_len = flash.max_app_len();
    let mut read_offset: usize = 0;
    header.validate(max_app_len, &mut |buf| {
        let n = flash.read_app(read_offset, buf).map_err(|_| ())?;
        read_offset += n;
        Ok(n)
    })?;

    Ok(header)
}
