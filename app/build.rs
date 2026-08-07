use std::path::PathBuf;

fn main() {
    // Build identity for the CAN GET_VERSION response. Rerun tracking is kept
    // narrow on purpose: a bare `../` would recursively stat `target/`.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../.git/HEAD");
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

        // When the "bootloader" feature is enabled, provide our own memory.x
        // that places the application at the bootloader app offset (0x08014800).
        // Without it, embassy-stm32's memory-x is used (full flash at 0x08000000).
        if std::env::var("CARGO_FEATURE_BOOTLOADER").is_ok() {
            std::fs::copy("../linker/app.x", out.join("memory.x")).unwrap();
            println!("cargo:rerun-if-changed=../linker/app.x");
        }
    }
}
