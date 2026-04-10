# EoI Rust Firmware

Embedded firmware for the STM32L471 microcontroller. Contains two application binaries that share common code (clock configuration, temperature sensor, CAN bus), a CAN bootloader for field updates, and a host-side flash tool. Communicates over CAN bus at 1 Mbps.

## Application Binaries

### Height Sensor Controller (`height-sensor-controller`)

- 4x height sensors via RS-485/Modbus
- Onboard temperature sensor via I2C
- CAN bus communication

### Rudder Controller (`rudder-controller`)

- Onboard temperature sensor via I2C
- CAN bus communication

## Bootloader (`eoi-boot`)

CAN-based bootloader that lives in the first 80K of flash. Allows firmware updates over CAN bus without a debug probe.

- Validates application on boot (header magic + CRC32)
- Auto-boots the application after 2 seconds if no CAN commands received
- Heartbeat LED double-flash pattern on PC1 (distinguishable from application's simple toggle)

## Flash Tool (`eoi-flash-tool`)

Host-side CLI tool for flashing firmware to the device over CAN bus (Linux SocketCAN).

## Flash Memory Layout

```
Region    Address Range               Size   Description
--------  --------------------------  -----  --------------------------
BOOT      0x08000000 - 0x08013FFF     80K    Bootloader
HEADER    0x08014000 - 0x080147FF     2K     Application metadata header
APP       0x08014800 - 0x080FFFFF     942K   Application firmware
RAM       0x20000000 - 0x20017FFF     96K    SRAM
```

All partition boundaries are aligned to the STM32L4 flash page size (2K).

## Bootloader Protocol

The bootloader communicates over CAN at 1 Mbps using standard 11-bit IDs.

**CAN IDs:** `0x030` (host to device commands), `0x031` (device to host responses), `0x032` (host to device write data)

**Host to device commands on `0x030`** (byte 0 = message type):

| Type | Name | Payload | Description |
| --- | --- | --- | --- |
| 0x01 | GetState | - | Request current state |
| 0x02 | EraseApp | - | Erase header + app partitions |
| 0x04 | ValidateApp | - | Validate app CRC |
| 0x05 | BootApp | - | Boot the application |
| 0x06 | Reboot | - | System reset |

**Host to device write data on `0x032`** (no type byte — all 8 bytes are payload):

| Payload | Description |
| --- | --- |
| 8 bytes | Sequential 8-byte aligned write to flash (header + app) |

**Device to host** (byte 0 = response type):

| Type | Name | Payload | Description |
| --- | --- | --- | --- |
| 0x01 | State | state: u8 | 0=WaitingNoApp, 1=WaitingWithApp, 2=Flashing |
| 0x02 | EraseOk | - | Erase complete |
| 0x03 | WriteAck | offset: u32 LE | Total bytes written so far |
| 0x04 | ValidateResult | result: u8 | 0=valid, 1=bad magic, 2=bad length, 3=bad CRC |
| 0x05 | BootAck | - | Will boot in 500ms |
| 0x06 | RebootAck | - | Will reboot |
| 0xFF | Error | code: u8 | Error response |

**Application header** (stored in 2K HEADER partition):

| Offset | Size | Field |
| --- | --- | --- |
| 0x00 | 4 | Magic: `[0xB0, 0x07, 0xCA, 0xFE]` |
| 0x04 | 1 | Header version (currently 1) |
| 0x05 | 4 | App length (LE u32) |
| 0x09 | 4 | App CRC32-ISCSI (LE u32) |
| 0x0D | ... | Padding (0xFF) to 2048 bytes |

**Update flow:** GetState -> EraseApp -> WriteData x N on `0x032` (header + app, 8 bytes per frame) -> ValidateApp -> BootApp

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
# Application binaries for development (full flash, no bootloader offset)
cargo build --release --bin height-sensor-controller
cargo build --release --bin rudder-controller

# Application binaries for use with bootloader (linked at app offset 0x08014800)
cargo build --release --bin height-sensor-controller --features bootloader
cargo build --release --bin rudder-controller --features bootloader

# Bootloader
cargo build --release -p eoi-boot

# Flash tool (host-side, must be built from its directory)
cd flash-tool && cargo build && cd ..
```

### 4. Flash via debug probe

Connect a debug probe (e.g. ST-Link) to the STM32L471 board.

Flash the bootloader first (only needed once):

```sh
cargo run --release -p eoi-boot
```

Then flash the application:

```sh
cargo run --release --bin height-sensor-controller
cargo run --release --bin rudder-controller
```

This will compile, flash the firmware onto the chip, and show log output via defmt.

### 5. Flash via CAN bus (using bootloader)

Once the bootloader is on the device, you can update the application over CAN without a debug probe.

Set up the CAN interface on the host (Linux):

```sh
sudo ip link set can0 type can bitrate 1000000
sudo ip link set can0 up
```

Build the firmware with the bootloader offset and flash it:

```sh
cargo build --release --bin rudder-controller --features bootloader
cd flash-tool
cargo run -- flash ../target/thumbv7em-none-eabi/release/rudder-controller
```

Other commands:

```sh
cargo run -- state                    # Read bootloader state
cargo run -- erase                    # Erase application
cargo run -- boot                     # Boot the application
cargo run -- reboot                   # Reboot into bootloader
cargo run -- flash --no-start FILE    # Flash without auto-booting
cargo run -- -i can1 flash FILE       # Use a different CAN interface
```
