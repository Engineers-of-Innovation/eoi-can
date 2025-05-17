fn main() {
    if std::env::var("TARGET").unwrap() != std::env::var("HOST").unwrap() {
        println!("cargo:rustc-link-arg-bins=--nmagic");
        println!("cargo:rustc-link-arg-bins=-Tlink.x");
        println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
    }
}
