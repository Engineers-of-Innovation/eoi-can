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

### Dashboard (`dashboard`)

Same board as the height sensor controller, with a Waveshare 5.79" e-paper display (792x272, SSD1683) on SPI2 instead of the RS-485 height sensors.

- Listens on CAN and renders the bus state to the e-paper panel
- Originates no bus traffic: the only frames it sends are replies to the host's bootloader-protocol queries. It has no onboard temperature sensor (SPI2's only DMA pair is the one I2C2 would need)
- Full panel refresh every 60th repaint, differential refresh otherwise; identical frames are skipped

Rendering lives in the `draw-display` crate in this repo, shared in spirit with the simulator and framebuffer tools in the `eoi-can` repo.

## Bootloader (`eoi-boot`)

CAN-based bootloader that lives in the first 80K of flash. Allows firmware updates over CAN bus without a debug probe.

The bootloader is compiled for exactly one board variant — select it with a cargo feature (`rudder-controller`, `height-sensor-controller` or `dashboard`). It refuses to boot an application whose header app type doesn't match.

- Validates application on boot (header magic + CRC32)
- Auto-boots the application 2 seconds after the last command addressed to this board
- Hardware CAN filter accepts only the discovery ID and this board's own IDs
- Heartbeat LED double-flash pattern on PC2 (distinguishable from application's simple toggle)

## Flash Tool (`eoi-flash-tool`)

Host-side CLI tool for flashing firmware to the boards over CAN bus (Linux SocketCAN).

Every command except `scan` is addressed to one board. `flash` takes its target from the
ELF's app type; the others need `--board`:

```sh
eoi-flash-tool scan                            # what is on the bus?
eoi-flash-tool flash ~/dashboard               # target comes from the ELF
eoi-flash-tool --board dashboard version       # which build is on it?
eoi-flash-tool --board dashboard reboot        # reset just that board
```

`scan`, `state` and `version` work whether a board is sitting in its bootloader or running its
application — both answer those three. `flash` reboots a running board into the bootloader itself.

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

The bootloader communicates over CAN at 1 Mbps using standard 11-bit IDs, all within `0x030`–`0x03F`.

Each board type owns three IDs, derived from its app type, so a command can only ever reach the
board it is addressed to. The bootloader's hardware filter rejects the other blocks outright — this
is what keeps an `EraseApp` aimed at one board from wiping the others. Every ID has exactly one
transmitter, including discovery replies, so two boards never contend for the same identifier.

**CAN IDs:** `base = 0x031 + (app_type - 1) * 3`

| Board | App type | Command (host→board) | Response (board→host) | Write data (host→board) |
| --- | --- | --- | --- | --- |
| rudder-controller | 0x01 | `0x031` | `0x032` | `0x033` |
| height-sensor-controller | 0x02 | `0x034` | `0x035` | `0x036` |
| dashboard | 0x03 | `0x037` | `0x038` | `0x039` |
| _(spare)_ | 0x04 | `0x03A` | `0x03B` | `0x03C` |
| _(spare)_ | 0x05 | `0x03D` | `0x03E` | `0x03F` |

`0x030` is the discovery broadcast: the host sends `GetState` on it and every bootloader answers on
its **own** response ID, which is how `eoi-flash-tool scan` enumerates the bus. Answering discovery
does not extend a board's auto-boot window, so repeated scanning cannot pin boards in the bootloader.

A sixth app type would overflow the block and has to extend the allocation into `0x040`+;
`board_address()` in `boot-api` asserts this at compile time.

**Host to device commands on the board's command ID** (byte 0 = message type):

| Type | Name | Payload | Description |
| --- | --- | --- | --- |
| 0x01 | GetState | - | Request current state |
| 0x02 | EraseApp | - | Erase header + app partitions |
| 0x04 | ValidateApp | - | Validate app CRC |
| 0x05 | BootApp | - | Boot the application |
| 0x06 | Reboot | - | System reset |
| 0x07 | GetVersion | - | Request the build identity of whatever is running |

`GetState` and `GetVersion` are answered by the bootloader _and_ by a running application, on the
board's command ID and on the discovery broadcast. The rest need the bootloader; a running
application rejects them with `Error` code `0x06`.

**Host to device write data on the board's write data ID** (no type byte — all 8 bytes are payload):

| Payload | Description |
| --- | --- |
| 8 bytes | Sequential 8-byte aligned write to flash (header + app) |

**Device to host on the board's response ID** (byte 0 = response type):

| Type | Name | Payload | Description |
| --- | --- | --- | --- |
| 0x01 | State | state: u8, app_type: u8 | 0=WaitingNoApp, 1=WaitingWithApp, 2=Flashing, 3=AppRunning. `app_type` lets the host cross-check the board it reached. |
| 0x02 | EraseOk | - | Erase complete |
| 0x03 | WriteAck | offset: u32 LE | Total bytes written so far |
| 0x04 | ValidateResult | result: u8 | 0=valid, 1=bad magic, 2=bad length, 3=bad CRC, 4=wrong app type |
| 0x05 | BootAck | - | Will boot in 500ms |
| 0x06 | RebootAck | - | Will reboot |
| 0x07 | Version | major, minor, patch, git[3], flags | See below |
| 0xFF | Error | code: u8 | 1=erase failed, 2=bad write length, 3=write failed, 4=no valid app, 5=unknown command, 6=application running |

State 3 (`AppRunning`) can only come from an application — the bootloader owns 0..=2. It is how the
host tells the two apart on a single response ID, and what makes `flash` reboot a board instead of
erasing under a live application.

**Version response payload** (`0x07`, all 8 bytes used):

| Offset | Size | Field |
| --- | --- | --- |
| 0x00 | 1 | Message type `0x07` |
| 0x01 | 3 | Version major, minor, patch (from the crate's Cargo.toml) |
| 0x04 | 3 | First three bytes of the commit hash (six hex chars) |
| 0x07 | 1 | Flags: bit0 = working tree dirty, bit1 = the bootloader answered, bit2 = not a git checkout |

Bootloader and application report their own build separately: the bootloader can only be replaced
over SWD, so it routinely sits several commits behind the application it boots, and the firmware
header carries no commit for it to read. Bit1 says which one you are looking at.

**Application header** (stored in 2K HEADER partition):

| Offset | Size | Field |
| --- | --- | --- |
| 0x00 | 4 | Magic: `[0xB0, 0x07, 0xCA, 0xFE]` |
| 0x04 | 1 | Header version (currently 1) |
| 0x05 | 4 | App length (LE u32) |
| 0x09 | 4 | App CRC32-ISCSI (LE u32) |
| 0x0D | 1 | App type: 0x01=rudder controller, 0x02=height sensor controller, 0x03=dashboard |
| 0x0E | ... | Padding (0xFF) to 2048 bytes |

**Update flow:** GetState (confirm the app type matches the image — this happens _before_ the
destructive erase) -> EraseApp -> WriteData x N on the board's write data ID (header + app, 8 bytes
per frame) -> ValidateApp -> BootApp

A running application answers `GetState` with `AppRunning` and `GetVersion` with its own build, so
`scan` enumerates the whole bus regardless of what each board is running and `version` works in
either mode. Everything destructive still needs the bootloader: the flash tool sends Reboot to the
board's command ID and waits for it. Applications use an accept-all filter (the dashboard needs the
whole bus), so `handle_bootloader_command` scopes the reset by checking the command ID against the
app's own type — rebooting one board leaves the others running.

`0x030` is the only ID an application answers that is not derived from its own app type, and it
replies there on its own response ID, so the one-transmitter-per-ID rule still holds.

> **Upgrading from the unaddressed protocol.** Bootloaders built before this addressing scheme listen
> on the flat `0x030`/`0x031`/`0x032`, where `0x032` is now the rudder controller's response ID. An
> old bootloader on the bus would read those responses as firmware write data. The bootloader can
> only be replaced over SWD, so re-flash every board with
> `cargo run --release -p eoi-boot --features <board>` before using the new flash tool on a shared
> bus.

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
cargo build --release --bin dashboard

# Application binaries for use with bootloader (linked at app offset 0x08014800)
cargo build --release --bin height-sensor-controller --features bootloader
cargo build --release --bin rudder-controller --features bootloader
cargo build --release --bin dashboard --features bootloader

# Bootloader (exactly one board variant feature must be enabled)
cargo build --release -p eoi-boot --features height-sensor-controller
cargo build --release -p eoi-boot --features rudder-controller
cargo build --release -p eoi-boot --features dashboard

# Flash tool (host-side, must be built from its directory)
cd flash-tool && cargo build && cd ..
```

Everything that crosses a crate boundary is pinned by host-side tests, because the bootloader can
only be replaced over SWD — a format the flash tool and the bootloader disagree about would brick a
board with no way back:

- `boot-api/tests/header.rs` — the firmware header byte layout, the CRC algorithm, and every
  rejection path (erased flash, bad magic, wrong header version, corrupt or truncated app)
- `boot-api/tests/protocol.rs` — the CAN ID allocation, the version frame, and how a running
  application answers each command (notably: reboot fires only on its own command ID)
- `flash-tool` — state decoding and how discovery pairs each board's state with its version
- `draw-display` — panel layout

The workspace defaults to the embedded target and `draw-display`'s `std` support is opt-in, so both
have to be named explicitly:

```sh
cargo test -p draw-display --features std --target x86_64-unknown-linux-gnu
cargo test -p eoi-boot-api --target x86_64-unknown-linux-gnu
cd flash-tool && cargo test && cd ..
```

### 4. Flash via debug probe

Connect a debug probe (e.g. ST-Link) to the STM32L471 board.

Flash the bootloader first (only needed once), with the feature matching the board:

```sh
cargo run --release -p eoi-boot --features height-sensor-controller
```

Then flash the application:

```sh
cargo run --release --bin height-sensor-controller
cargo run --release --bin rudder-controller
cargo run --release --bin dashboard
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
cargo run -- flash ../target/thumbv7em-none-eabihf/release/rudder-controller
```

`flash` needs no `--board`: the target comes from the ELF's app type, so pointing it at a file can
only ever address the board that file was built for. Boards other than the target keep running
untouched.

Other commands are addressed explicitly with `--board`:

```sh
cargo run -- scan                                    # List boards on the bus
cargo run -- --board dashboard state                 # Read state (works while the app runs)
cargo run -- --board dashboard version               # Read version + git hash
cargo run -- --board dashboard erase                 # Erase application
cargo run -- --board dashboard boot                  # Boot the application
cargo run -- --board dashboard reboot                # Reboot into bootloader
cargo run -- flash --no-start FILE                   # Flash without auto-booting
cargo run -- -i can1 flash FILE                      # Use a different CAN interface
```

To build and ship to a Raspberry Pi on the boat, `./build-and-send.sh <user@host> [board]` sends the
flash tool plus one board's firmware (or all three if no board is named).
