use std::path::PathBuf;

fn main() {
    if std::env::var("TARGET").unwrap() != std::env::var("HOST").unwrap() {
        println!("cargo:rustc-link-arg-bins=--nmagic");
        println!("cargo:rustc-link-arg-bins=-Tlink.x");
        println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

        // When the "bootloader" feature is enabled, provide our own memory.x
        // that places the application at the bootloader app offset (0x08014800).
        // Without it, embassy-stm32's memory-x is used (full flash at 0x08000000).
        if std::env::var("CARGO_FEATURE_BOOTLOADER").is_ok() {
            let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
            std::fs::copy("../linker/app.x", out.join("memory.x")).unwrap();
            println!("cargo:rustc-link-search={}", out.display());
            println!("cargo:rerun-if-changed=../linker/app.x");
        }
    }
}
