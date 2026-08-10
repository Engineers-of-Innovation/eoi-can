use std::path::PathBuf;

fn main() {
    // Build identity for the CAN GET_VERSION response. The bootloader is
    // flashed separately from the app, so it reports its own commit, not the
    // app's. Rerun tracking stays narrow — a bare `../` would stat `target/`.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    built::write_built_file().expect("Failed to acquire build-time information");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::copy("../linker/boot.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=../linker/boot.x");

    // Linker arguments for the bootloader binary
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
