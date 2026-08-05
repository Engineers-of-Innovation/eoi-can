# EoI Rust Firmware

Embedded firmware for the STM32L471 microcontroller. Contains two application binaries that share common code (clock configuration, temperature sensor, CAN bus), a CAN bootloader for field updates, and a host-side flash tool. Communicates over CAN bus at 1 Mbps.

## Application Binaries

### Height Sensor Controller (`height-sensor-controller`)

- 4x height sensors via RS-485/Modbus
- Onboard temperature sensor via I2C
- CAN bus communication

### Rudder Controller (`rudder-controller`)

- Onboard temperature sensor via I2C
- Steering angle sensor with persistent calibration (see below)
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
APP       0x08014800 - 0x080FEFFF     938K   Application firmware
CONFIG    0x080FF000 - 0x080FFFFF     4K     Emulated EEPROM (persistent config)
RAM       0x20000000 - 0x20017FFF     96K    SRAM
```

All partition boundaries are aligned to the STM32L4 flash page size (2K).

The STM32L471 has no data EEPROM, so the CONFIG block emulates one. It sits
outside the app partition, so the bootloader's erase stops short of it and a
firmware update over CAN leaves stored configuration intact. It is also in
flash bank 2 while code executes from bank 1, so erasing and programming it
never stalls the CPU.

The block is an append-only log of 16-byte slots, each carrying a magic,
version and CRC32. The newest slot that validates wins on load; when all 256
slots are used the block is erased and writing restarts at slot 0.

Note: a bootloader flashed before the CONFIG block existed erases the full
942K app partition and so wipes the block. That is safe — the configuration
reads back as uninitialized, the affected outputs fall back to their zero state
and set an error bit, and re-running calibration fixes it. Reflash the
bootloader via a debug probe to avoid it.

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

## Steering Angle Calibration

The steering angle sensor is a potentiometer on ADC1. It has no fixed
raw-to-angle mapping, so the three reference positions are calibrated on the
vehicle and stored in the CONFIG block.

Calibration is captured live — no values travel over the bus. Move the steering
to a position, then send the matching command; the firmware averages readings
over 200 ms and stores the result immediately, so a session can be interrupted
and resumed. Hold the steering still for that 200 ms.

**Reported position** on `0x213` is normalized against the calibrated travel and
is piecewise-linear through the calibrated centre:

| Steering        | Position |
| --------------- | -------- |
| Full left       | -1000    |
| Centre          | 0        |
| Full right      | +1000    |

**Steering angle broadcast on `0x213`** (10 Hz):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 2 | Position, i16 LE (-1000..+1000), 0 when not calibrated |
| 2 | 2 | Averaged raw ADC code, u16 LE (still 12-bit, 0..4095) |
| 4 | 1 | Status bits (see below) |

**Noise rejection.** The sensor is electrically noisy, so the ADC runs much
faster than the 10 Hz report rate and the readings are averaged. Two layers,
because either alone leaves a gap:

- The ADC hardware averages 16 conversions per read and right-shifts back to the
  same 12-bit scale. This suppresses white noise 4x, but the burst spans only
  ~52 us, so it does nothing for interference slower than roughly 20 kHz.
- 20 reads are spread evenly across each 100 ms reporting window (one every
  5 ms) and box-averaged. Averaging over exactly one window puts nulls at 10 Hz
  and every harmonic, which is what rejects periodic interference from the
  stepper driver and cooling pump.

Together that is 320 conversions per report for ~1 % ADC duty cycle, and about
18x white-noise rejection. Group delay is half the window (50 ms), inherent to
averaging over it. Calibration captures average 40 reads spread over 200 ms,
so an endpoint is never set by a noisy moment.

**Status bits** (byte 4 of `0x213`, byte 5 of `0x218`):

| Bit | Name | Meaning |
| --- | --- | --- |
| 0 | CalValid | Calibration present and plausible; position is meaningful |
| 1 | CalMissing | Nothing stored yet, or every stored record is corrupt |
| 2 | CalInvalid | Stored but incomplete, or implausible (see rejection rules) |
| 3 | OutOfRange | Raw reading is outside the calibrated travel; position clamped |
| 4 | StorageError | Last write to the CONFIG block failed; sticky until one succeeds |
| 5 | NotConnected | Presence pin on the sensor connector reads high; no sensor plugged in |

Whenever CalValid is clear the reported position is held at 0, regardless of
the sensor reading.

**Sensor presence.** PB2 on the sensor connector is an input with an internal
pull-up, rather than the LED feedback output it used to drive. The sensor pulls
the pin to ground, so a high reading means the connector is empty and
NotConnected is set. It is sampled once per report and is independent of the
calibration bits: a stored calibration stays valid while the sensor is
unplugged.

**Calibration commands on `0x214`** (byte 0 = command, remaining bytes ignored):

| Command | Name | Description |
| --- | --- | --- |
| 0x01 | CaptureLeft | Store the current reading as the full-left endpoint |
| 0x02 | CaptureCenter | Store the current reading as the centre |
| 0x03 | CaptureRight | Store the current reading as the full-right endpoint |
| 0x04 | Clear | Discard the calibration and return to the uncalibrated safe state |

**Calibration ack on `0x218`** (sent for every accepted command):

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | Echoed command |
| 1 | 1 | Result: 0=ok, 1=storage error, 2=stored but set not yet usable |
| 2 | 2 | Captured raw ADC code, u16 LE (0 for Clear) |
| 4 | 1 | Captured-endpoint bits: 0x01=left, 0x02=centre, 0x04=right |
| 5 | 1 | Status bits (same encoding as `0x213` byte 4) |

**A calibration is rejected** (CalInvalid, position forced to 0) when any of the
three endpoints has not been captured, when a raw code exceeds 4095, when the
centre does not lie strictly between the two endpoints, or when either
half-travel is narrower than 100 ADC codes. Both wiring polarities are
accepted: full-left may read either above or below full-right.

**Calibration procedure:** steer fully left -> send `0x01`, centre -> send
`0x02`, steer fully right -> send `0x03`. Order does not matter. The set becomes
active on the next 10 Hz sample once all three are captured and plausible.

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

`cargo run` goes through `tools/flash-jlink.sh`, which picks a backend
automatically:

| Platform | Backend | Notes |
| --- | --- | --- |
| Linux, macOS | `probe-rs` | Flashes and streams defmt logs |
| WSL | Windows-side J-Link Commander | probe-rs cannot reach a USB probe from WSL; flashes and runs, but no log streaming |

Force one with `FLASH_BACKEND=probe-rs` or `FLASH_BACKEND=jlink` — for example on
WSL with a probe attached via `usbipd`:

```sh
FLASH_BACKEND=probe-rs cargo run --release --bin rudder-controller
```

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
