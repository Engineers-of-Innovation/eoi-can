pub use eoi_boot_api::header::AppType;

/// Declare the application type for a binary. Emits a 1-byte symbol in the
/// `.app_type` ELF section, which the flash tool reads to stamp the firmware
/// header. The section is NOLOAD (see linker/app.x), so it does not occupy
/// flash on the device.
///
/// Also defines `MY_APP_TYPE` for runtime use — the app needs its own type to
/// work out which bootloader CAN address belongs to it, so that a `REBOOT`
/// aimed at another board does not reset this one.
#[macro_export]
macro_rules! declare_app_type {
    ($t:expr) => {
        #[used]
        #[unsafe(link_section = ".app_type")]
        static APP_TYPE: u8 = $t as u8;

        const MY_APP_TYPE: $crate::app_type::AppType = $t;
    };
}
