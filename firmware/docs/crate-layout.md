# Firmware crate layout

Five crates, and the split is driven by one constraint: **the MCU is a Cargo
feature, and a Cargo feature cannot differ per binary within a package.**

| Crate | What it is | MCU |
| --- | --- | --- |
| `common` | Every shared module: CAN, clock, config, display, sensors, actuators | *none — inherited* |
| `app` | The rudder controller, height sensor controller and helm dashboard binaries | `stm32l471rg` |
| `app-foiling` | The foiling display binary | `stm32l476rg` |
| `boot` | Bootloader, built once per board via its own features | `stm32l471rg` |
| `boot-api` | Header and CAN protocol types, shared with the host flash tool | host + target |
| `build-support` | Build-script linker wiring shared by the application crates | – |

`common` names **no** chip feature. Each application crate names its own, and
Cargo's feature unification applies it to `common`'s copy of `embassy-stm32`.

## Building

Build one application crate at a time:

```sh
cargo build --release -p eoi-firmware --bins --features bootloader   # the L471 boards
cargo build --release -p eoi-firmware-foiling                        # the L476 board
```

Two invocations are not a style choice. `stm32-metapac`'s build script rejects
anything else:

- `cargo build -p eoi-firmware -p eoi-firmware-foiling` → *Multiple stm32xx Cargo
  features enabled*
- `cargo build -p eoi-firmware-common` → *No stm32xx Cargo feature enabled*

So there is no `--workspace` build of this directory, and `default-members` is
`["app"]` so a bare `cargo build` stays meaningful. For rust-analyzer, point it
at one application crate; `common` alone will not check.

Note that switching between the two crates rebuilds `embassy-stm32` and
`stm32-metapac`, because the chip feature differs. That is a few seconds, not a
correctness problem.

## Display boards

The helm dashboard and the foiling display are the same board design and run the
same decode pipeline. `common::display::run_display` owns the pin choices and the
refresh loop; each binary is only a `bind_interrupts!`, an app type, and a
`draw_display::Layout`. Adding a screen should not mean touching either binary.

`bind_interrupts!` has to stay in the binaries — it emits `#[no_mangle]`
handlers, and a single copy in `common` would collide with the `CAN1_*` handlers
the rudder and height-sensor binaries declare for themselves.

Both layouts are linked into both images, because `Layout::draw` matches over the
enum. At ~99 KB of a 1 MB part that is not worth avoiding; if flash ever gets
tight, gate the layout behind a feature so LTO can drop the unused one.

## MCU differences that matter

L476RG is a peripheral superset of L471RG with the same 1 MB flash, same 128 KB
RAM and the same LQFP64 pinout, so `linker/app.x` and `linker/app-dev.x` apply
unchanged and `config.rs`'s flash-top config block lands in the same place.

The one pinout difference is **pin 48: VDD on the L471, VDDUSB on the L476**. The
board ties it to 3v3, which is in range for both (`VDDUSB` is 3.0–3.6 V when USB
is used, 0–3.6 V when it is not). Because VDDUSB is powered rather than grounded,
PA9–PA12 also keep full 5 V tolerance, which matters because PA12 carries
`UART_DETECT4` on the display board.

## TODO: bootloader on the foiling board

Deferred deliberately. The foiling board is flashed over SWD (`app-foiling` has
`bootloader` off by default, so it links flat at `0x08000000`), and nothing
answers on its bootloader address on the bus.

To deploy CAN OTA on it later:

1. Add a `foil-tuning` feature to `boot` selecting `AppType::FoilTuning`, which
   already exists.
2. Add `FoilTuning` to the flash tool's `Board` enum — left out on purpose today
   so the CLI cannot offer a target that only times out.
3. Build `app-foiling` with `--features bootloader`.
4. Check `linker/boot.x`: `__app_end` is hardcoded to `0x080FF000`. Correct for
   1 MB, so an L476RG needs no change — but it *would* have needed one for the
   512 KB L471RE that was briefly the plan, so verify against the fitted part.

**Never let two boards on the bus report the same app type.** The app type *is*
the bootloader's CAN address, so a `REBOOT` or a flash aimed at one would hit
both, and the two would answer over each other.
