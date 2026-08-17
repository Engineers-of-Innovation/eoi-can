fn main() {
    // Build identity for the CAN GET_VERSION response and the boot-time defmt
    // line, consumed by `src/build_info.rs`. Rerun tracking is kept narrow on
    // purpose: a bare `../` would recursively stat `target/`.
    //
    // The git directory is the repo root's, two levels up -- there is no
    // `firmware/.git`. (This said `../.git/HEAD` while this file lived in
    // `app/`, which existed even less.)
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    built::write_built_file().expect("Failed to acquire build-time information");
}
