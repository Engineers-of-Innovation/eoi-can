use std::path::PathBuf;

fn main() {
    // Build identity for the CAN GET_VERSION response. Rerun tracking is kept
    // narrow on purpose: a bare `../` would recursively stat `target/`. The git
    // directory is the repo root's, two levels up — there is no `firmware/.git`.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    built::write_built_file().expect("Failed to acquire build-time information");

    if std::env::var("TARGET").unwrap() != std::env::var("HOST").unwrap() {
        println!("cargo:rustc-link-arg-bins=--nmagic");
        println!("cargo:rustc-link-arg-bins=-Tlink.x");
        println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

        let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        println!("cargo:rustc-link-search={}", out.display());

        // Place the .app_type metadata in a non-loaded ELF section so the
        // flash tool can read it without the byte ending up in flash.
        std::fs::copy("../linker/app-type.x", out.join("app-type.x")).unwrap();
        println!("cargo:rustc-link-arg-bins=-Tapp-type.x");
        println!("cargo:rerun-if-changed=../linker/app-type.x");

        // Provide our own memory.x in place of embassy-stm32's memory-x. With
        // the "bootloader" feature the application is placed at the bootloader
        // app offset (0x08014800); without it, it starts at 0x08000000. Both
        // reserve the emulated-EEPROM block at the top of flash so the linker
        // can never place code over it.
        //
        // The feature is crate-wide, so it applies to every binary in one
        // invocation. The foiling image is built without it, in its own
        // invocation, because that board has no bootloader — see
        // `docs/crate-layout.md`.
        let memory_x = if std::env::var("CARGO_FEATURE_BOOTLOADER").is_ok() {
            "../linker/app.x"
        } else {
            "../linker/app-dev.x"
        };
        std::fs::copy(memory_x, out.join("memory.x")).unwrap();
        println!("cargo:rerun-if-changed={memory_x}");
    }
}
