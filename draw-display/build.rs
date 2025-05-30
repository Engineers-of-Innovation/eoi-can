fn main() {
    println!("cargo:rerun-if-changed=../");
    built::write_built_file().expect("Failed to acquire build-time information");
}
