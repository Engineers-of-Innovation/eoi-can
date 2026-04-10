# EoI Rust Firmware

Embedded firmware for the STM32L471 microcontroller. Contains two binaries that share common code (clock configuration, temperature sensor, CAN bus). Communicates over CAN bus at 1 Mbps.

## Height Sensor Controller (`height-sensor-controller`)

- 4x height sensors via RS-485/Modbus
- Onboard temperature sensor via I2C
- CAN bus communication

## Rudder Controller (`rudder-controller`)

- Onboard temperature sensor via I2C
- CAN bus communication

## Getting Started

This project is written in Rust for an embedded (bare-metal) ARM target. If you don't have Rust installed yet, follow these steps.

### 1. Install Rust

Install Rust using rustup:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Follow the on-screen instructions and restart your terminal afterwards.

### 2. Install tools

Install `flip-link` (linker wrapper for embedded) and `probe-rs` (flash/debug tool):

```sh
cargo install flip-link
cargo install probe-rs-tools
```

### 3. Build

```sh
cargo build --release                              # both binaries
cargo build --release --bin height-sensor-controller
cargo build --release --bin rudder-controller
```

### 4. Flash and run

Connect a debug probe (e.g. ST-Link) to the STM32L471 board, then:

```sh
cargo run --release --bin height-sensor-controller
cargo run --release --bin rudder-controller
```

This will compile, flash the firmware onto the chip, and show log output via defmt.
