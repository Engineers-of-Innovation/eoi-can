//! Build identity of this bootloader image, for the CAN `GET_VERSION` response
//! and the boot-time defmt line.
//!
//! Deliberately separate from the application's copy (`app/src/build_info.rs`):
//! the bootloader can only be replaced over SWD, so it routinely sits several
//! commits behind the app it boots, and reporting the app's commit here would
//! be a lie.

use eoi_boot_api::protocol::VersionInfo;

mod built {
    // Placed there by build.rs via the `built` crate.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// What this image reports over CAN. `const`, so the string parsing folds away
/// at compile time and none of it reaches the 80K BOOT partition.
pub const VERSION: VersionInfo = VersionInfo::from_built(
    built::PKG_VERSION_MAJOR,
    built::PKG_VERSION_MINOR,
    built::PKG_VERSION_PATCH,
    built::GIT_COMMIT_HASH,
    built::GIT_DIRTY,
    // The bootloader, not the application.
    true,
);

/// Short commit hash for logging. Matches the six hex chars [`VERSION`] puts on
/// the wire.
pub const GIT_HASH_SHORT: &str = match built::GIT_COMMIT_HASH {
    Some(h) if h.len() >= 6 => h.split_at(6).0,
    Some(h) => h,
    None => "unknown",
};

/// `PKG_VERSION` as written in Cargo.toml.
pub const PKG_VERSION: &str = built::PKG_VERSION;

/// Log the build identity, so a bootloader is identifiable over RTT as well as
/// over CAN.
pub fn log() {
    defmt::info!(
        "Bootloader build: v{} git {}{}",
        PKG_VERSION,
        GIT_HASH_SHORT,
        if VERSION.dirty { "-dirty" } else { "" }
    );
}
