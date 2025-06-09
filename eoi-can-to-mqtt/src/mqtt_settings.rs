pub const BROKER: &str = "ssl://git.engineersofinnovation.nl:8883";
#[cfg(debug_assertions)]
pub const CLIENT: &str = "dev_rust_publish";
#[cfg(not(debug_assertions))]
pub const CLIENT: &str = "rust_publish";
pub const USER: &str = "engineer";
pub const PASSWORD: &str = "EoI-42";
pub const TRUST_STORE: &str = "certs/isrgrootx1.pem";
pub const TOPIC: &str = "eoi-can-to-mqtt";
