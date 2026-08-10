//! Build identity of this application image, for the CAN `GET_VERSION`
//! response and the boot-time defmt line.
//!
//! The bootloader has its own copy of this (`boot/src/build_info.rs`) rather
//! than sharing one: the two are flashed independently and can be built from
//! different commits, so each has to report its own.

use eoi_boot_api::protocol::VersionInfo;

mod built {
    // Placed there by build.rs via the `built` crate.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// What this image reports over CAN. `const`, so the string parsing folds away
/// at compile time and none of it reaches flash.
pub const VERSION: VersionInfo = VersionInfo::from_built(
    built::PKG_VERSION_MAJOR,
    built::PKG_VERSION_MINOR,
    built::PKG_VERSION_PATCH,
    built::GIT_COMMIT_HASH,
    built::GIT_DIRTY,
    // The application, not the bootloader.
    false,
);

/// Short commit hash for logging. Matches the six hex chars [`VERSION`] puts on
/// the wire, so a defmt line and a `eoi-flash-tool version` agree.
pub const GIT_HASH_SHORT: &str = match built::GIT_COMMIT_HASH {
    Some(h) if h.len() >= 6 => h.split_at(6).0,
    Some(h) => h,
    None => "unknown",
};

/// `PKG_VERSION` as written in Cargo.toml.
pub const PKG_VERSION: &str = built::PKG_VERSION;

/// Log the build identity. Every binary calls this right after `init`, so an
/// image is identifiable over RTT as well as over CAN.
pub fn log() {
    defmt::info!(
        "Build: v{} git {}{}",
        PKG_VERSION,
        GIT_HASH_SHORT,
        if VERSION.dirty { "-dirty" } else { "" }
    );
}
