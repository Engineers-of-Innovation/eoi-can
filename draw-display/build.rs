fn main() {
    // Narrow rerun tracking to what actually feeds the build. A `../` here
    // would cover the whole workspace root, which means recursively stat'ing
    // `target/` on every build.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    built::write_built_file().expect("Failed to acquire build-time information");
}
