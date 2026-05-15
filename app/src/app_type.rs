pub use eoi_boot_api::header::AppType;

/// Declare the application type for a binary. Emits a 1-byte symbol in the
/// `.app_type` ELF section, which the flash tool reads to stamp the firmware
/// header. The section is NOLOAD (see linker/app.x), so it does not occupy
/// flash on the device.
#[macro_export]
macro_rules! declare_app_type {
    ($t:expr) => {
        #[used]
        #[unsafe(link_section = ".app_type")]
        static APP_TYPE: u8 = $t as u8;
    };
}
