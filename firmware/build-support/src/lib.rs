//! Linker wiring for the application crates, called from their `build.rs`.
//!
//! Every application board shares one flash layout, so it is defined once here
//! and in `firmware/linker/`. Each application crate needs its own call because
//! `cargo:rustc-link-arg-bins` only applies to binaries in the emitting
//! package — a dependency cannot emit it on their behalf.

use std::path::{Path, PathBuf};

/// Emit the link arguments and the `memory.x` that the application binaries in
/// the calling package need.
///
/// Selects the flash layout from the calling crate's `bootloader` feature: with
/// it, the image is linked at the bootloader's application offset
/// (`0x08014800`); without it, at `0x08000000` for a direct SWD flash. Both
/// reserve the emulated-EEPROM block at the top of flash so the linker can
/// never place code over it.
///
/// A no-op for host builds, where there is no embedded target to link for.
pub fn configure_app_linking() {
    println!("cargo:rerun-if-changed=src");

    let host = std::env::var("HOST").expect("cargo sets HOST");
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    if target == host {
        return;
    }

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    println!("cargo:rustc-link-search={}", out.display());

    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let linker_dir = manifest.join("..").join("linker");

    // Place the .app_type metadata in a non-loaded ELF section so the flash tool
    // can read it without the byte ending up in flash.
    copy_into(&linker_dir.join("app-type.x"), &out.join("app-type.x"));
    println!("cargo:rustc-link-arg-bins=-Tapp-type.x");

    // Our own memory.x, in place of the one embassy-stm32's `memory-x` feature
    // would generate from the chip. Deliberate: the layout has to account for
    // the bootloader, the header and the config block, none of which the chip
    // feature knows about.
    let memory_x = if std::env::var_os("CARGO_FEATURE_BOOTLOADER").is_some() {
        "app.x"
    } else {
        "app-dev.x"
    };
    copy_into(&linker_dir.join(memory_x), &out.join("memory.x"));
}

fn copy_into(from: &Path, to: &Path) {
    std::fs::copy(from, to)
        .unwrap_or_else(|e| panic!("copying {} to {}: {e}", from.display(), to.display()));
    println!("cargo:rerun-if-changed={}", from.display());
}
