# EoI Firmware

Embedded firmware for the STM32L471 microcontroller. Contains four application binaries that share common code (clock configuration, temperature sensor, CAN bus), a CAN bootloader for field updates, and a host-side flash tool. Communicates over CAN bus at 1 Mbps.

## Application Binaries

### Height Sensor Controller (`height-sensor-controller`)

- 4x height sensors via RS-485/Modbus
- Onboard temperature sensor via I2C
- CAN bus communication

### Rudder Controller (`rudder-controller`)

- Onboard temperature sensor via I2C
- Steering angle sensor with persistent calibration (see below)
- CAN bus communication

### Dashboard (`dashboard`)

Same board as the height sensor controller, with a Waveshare 5.79" e-paper display (792x272, SSD1683) on SPI2 instead of the RS-485 height sensors.

- Listens on CAN and renders the bus state to the e-paper panel
- Originates no bus traffic: the only frames it sends are replies to the host's bootloader-protocol queries. It has no onboard temperature sensor (SPI2's only DMA pair is the one I2C2 would need)
- Full panel refresh every 60th repaint, differential refresh otherwise; identical frames are skipped

Rendering lives in the [`draw-display`](../draw-display) crate at the repo root, shared with the simulator and framebuffer tools.

### Motor NTC Sensor (`motor-ntc-sensor`)

Rudder-controller hardware doing one job: read a 10 kΩ NTC on the steering
potentiometer input and broadcast the motor temperature on `0x219` at 1 Hz.
See [Motor NTC sensor](#motor-ntc-sensor-1) below.

- No bootloader, no persistent config, no servo, no pump — transmit only
- Wire-identical to the standalone [`can-motor-temperature`](https://github.com/Engineers-of-Innovation) node, so nothing downstream has to know which board is fitted

## Bootloader (`eoi-boot`)

CAN-based bootloader that lives in the first 80K of flash. Allows firmware updates over CAN bus without a debug probe.

The bootloader is compiled for exactly one board variant — select it with a cargo feature (`rudder-controller`, `height-sensor-controller` or `dashboard`). It refuses to boot an application whose header app type doesn't match. `motor-ntc-sensor` deliberately has no variant and no app type; it owns all of flash and is flashed with a probe.

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

## Motor NTC Sensor

The `motor-ntc-sensor` binary turns a rudder controller board into the standalone
motor temperature node, broadcasting `0x219` at 1 Hz in 0.1 °C. The frame and the
conversion chain are ported unchanged from the `can-motor-temperature` firmware
(STM32G491 on CANable 2.5 hardware), so the two are interchangeable on the wire —
see the `MotorNtc` rows in [CAN_MESSAGES.md](../CAN_MESSAGES.md) for the layout and
the status bits.

**Only one of the two may be on a bus at a time.** They share `0x219` and each
transmits unprompted; two transmitters on one identifier is a collision, not a
redundancy.

### Wiring

```
      PB2 ─────[ 10k ]───── PB1 ─────[ 47R ]─────[ 10k NTC ]───── GND
  pot_feedback               │  pot_analog_in
 (bias, switched)            └─ ADC1_IN16 + any filter capacitor to GND
```

| Function | Pin | Net | Notes |
| --- | --- | --- | --- |
| NTC sense | PB1 | `pot_analog_in` | ADC1_IN16, the steering potentiometer input |
| Divider bias | PB2 | `pot_feedback` | Push-pull, driven high only while measuring |

The 47 Ω is the resistor already on the board in series with the sensor leg; it is
subtracted in `motor_ntc.rs` (`R_SERIES_LOW_OHMS`). It is worth ~0.1 °C at room
temperature, so if it turns out to sit between the connector and the MCU pin rather
than in the divider leg, set that constant to 0 — an ADC input draws no DC, so a
resistor there divides nothing.

**Why bias from a GPIO rather than from 3V3.** The divider is fed from VDD through
PB2 and the ADC reference is VDDA, the same rail, so the supply voltage cancels out
of the ratio: no VREFINT correction, and supply ripple from the motor drive does not
appear in the reading. The NTC is also biased only ~215 ms per second, so
self-heating is far below 0.1 °C. Between measurements PB2 is driven **low**, not
left floating, so both ends of the divider sit at ground and the sense node cannot
be pumped around by capacitive coupling from the motor cables.

### Filtering

| Stage | What |
| --- | --- |
| analog | whatever filter capacitor is fitted on the sense node |
| ADC hardware oversampler | 256 accumulations per read, right-shifted 4 for a 16-bit result (full scale 65520), zero CPU cost |
| trimmed mean | 16 reads spread over 64 ms, sorted, highest 4 and lowest 4 **discarded** |
| IIR + slew limit | first-order IIR (shift 2, ~3 s), then at most 5.0 °C change per update |

The filtered code that comes out of this goes on the wire in bytes 4–5, which is
what makes a wiring fault diagnosable from a `candump` line alone — see
[Reading the raw code](#reading-the-raw-code).

4096 hardware samples per second, of which 2048 can be arbitrarily corrupted without
moving the result. The trimming is the part that matters next to motor cables: an
interference burst landing inside one read is thrown away outright rather than
averaged in, which a plain mean cannot do.

`BIAS_ALWAYS_ON` (currently `true`) decides whether the divider is switched per
measurement or left powered. Switched, one second looks like this:

| t | |
| --- | --- |
| 0 ms | divider bias on |
| 150 ms | ADC burst starts — 15 τ of settling for a 1 µF cap on a 10 kΩ source |
| 150–214 ms | 16 oversampled reads, ~1.7 ms each, spread 4 ms apart |
| 214 ms | trimmed mean, IIR, convert, transmit, bias off |

Leaving it on costs the divider's ~165 µA continuously and gives up the duty
cycling that keeps NTC self-heating negligible — ~0.27 mW in the NTC, tens of
millikelvin in still air. What it buys is a node you can put a multimeter on:
switched, the sense node only has a voltage on it for ~215 ms per second, and a
handheld meter reads the average of that, which is neither of the two states.

Unlike the standalone node this one does not sleep, and its period comes from the
80 MHz PLL rather than an LSI, so the rate is a solid 1 Hz rather than 1 Hz ±5 %.

### Reading the raw code

Bytes 4–5 carry the filtered ADC code, full scale 65520. `candump can0,219:7FF -x`
and read it back to a node voltage and a leg resistance:

```
ratio = code / 65520            V(PB1) = ratio * 3.3 V
Rlow  = 10030 * ratio / (1 - ratio)      # the whole low leg, 47R + NTC
```

| Code | ratio | V(PB1) | Low leg | Means |
| --- | --- | --- | --- | --- |
| 65520 | 1.000 | 3.30 V | ∞ | NTC or its ground return is open (`0x01`) |
| 63979 | 0.977 | 3.22 V | 416 kΩ | -40 °C, the cold clamp |
| 50616 | 0.773 | 2.55 V | 34 kΩ | 0 °C |
| 32788 | 0.500 | 1.65 V | 10 kΩ | 25 °C |
| 29165 | 0.445 | 1.47 V | 8.0 kΩ | 30 °C |
| 13073 | 0.200 | 0.66 V | 2.5 kΩ | 60 °C |
| 1527 | 0.023 | 0.08 V | 239 Ω | +150 °C, the hot clamp |
| 0 | 0.000 | 0.00 V | 0 Ω | NTC shorted, or the bias never rose (`0x02`) |

A code that disagrees with what a meter reads on PB1 is a firmware or ADC problem.
A code that agrees with the meter but not with the resistance you measured at the
connector is a wiring problem, and the table says which way.

### Differences from the standalone node

- **`0x10` (acquisition error) is never set.** The ADC is read synchronously here;
  there is no DMA burst that can fail to arrive.
- **`0x20` (CAN Tx failed) is weaker.** It means the previous frame could not be
  queued, not that it went unacknowledged — `BufferedCanSender` does not surface
  the acknowledge.
- **No bootloader, no low-power modes, no `GetVersion` over CAN.** `eoi-flash-tool
  scan` will not see this board; it reports its build over defmt at boot only.

### Bring-up

```sh
cargo run --release --bin motor-ntc-sensor
```

Under WSL that goes through the Windows-side J-Link Commander and flashes without
streaming logs; with the probe passed through by `usbipd attach --wsl`, force
`FLASH_BACKEND=probe-rs` to get defmt output as well. Note this image owns all of
flash, so flashing it over a board that already has the bootloader erases the
bootloader — re-flash `eoi-boot --features rudder-controller` to get it back.

With the divider unpopulated the sense node floats up to the bias rail, so a board
with no NTC fitted reports `0x8000` with status `0x01` (SensorOpen), once a second.
That is the correct answer and a good sign the loop is running. With the 10 kΩ
pull-up and a 10 kΩ NTC fitted, room temperature should read a filtered code near
32800 and about 25 °C.

### Other rudder controller hardware

Nothing else on the board is driven. The stepper enable (PB5, active-low) and the
cooling pump enable (PA6, active-high) are parked in their off state at boot rather
than left as inputs, so a board that is still wired to a motor or a pump cannot have
either come alive by accident.

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
cargo build --release --bin motor-ntc-sensor   # probe only, no bootloader variant

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
cargo run --release --bin motor-ntc-sensor
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
