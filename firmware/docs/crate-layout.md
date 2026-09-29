# Firmware layout, and why one image covers two MCUs

Three crates, and one non-obvious decision behind them.

| Crate | What it is |
| --- | --- |
| `app` | Every board's application: four binaries over one shared library |
| `boot` | Bootloader, built once per board via its own features |
| `boot-api` | Header and CAN protocol types, shared with the host flash tool |

`app` builds four binaries — `rudder-controller`, `height-sensor-controller`,
`dashboard`, `foiling` — from the modules in `app/src/`.

## The foiling board is a different MCU, and that is fine

The foiling display is an **STM32L476RG**; every other board is an
**STM32L471RG**. A supply-chain substitution, not a design change.

Everything is nonetheless compiled for `stm32l471rg`, and the foiling image runs
unmodified on the L476. The two parts are compatible to the point where the
entire difference in `stm32-metapac`'s generated PAC is:

- `OTG_FS = 67` and `LCD = 78` added to the interrupt enum,
- the `otg` and `lcd` register blocks,
- the two vector-table slots those fill, `_reserved` on the L471.

Verified identical between the two: every peripheral base address (CAN1, SPI2,
DMA1, IWDG, TIM4, RCC, FLASH, GPIOA-C, PWR, EXTI), every RCC enable/reset bit
position, SPI2's DMA channels and request numbers (`DMA1_CH4`/`CH5`, request 1),
every interrupt number these binaries bind, and `FLASH_SIZE` (1 MB). embassy's
own generated per-chip glue is a strict superset — not one line is unique to the
L471.

**Building for the subset is what makes this safe rather than lucky.** Compiled
against `stm32l471rg`, the PAC cannot expose USB or the LCD, so no binary can
come to depend on a peripheral the L471 boards lack; and the two vector slots
that stay `_reserved` belong to peripherals whose clocks can never be enabled,
so nothing can vector through them. Compiling for the L476 instead would work
today and rot the first time somebody reached for USB.

Three things follow:

1. **The moment the foiling board needs USB or the LCD, this ends.** That board
   then needs its own crate with its own chip feature, because a Cargo feature
   cannot differ per binary within a package. (That split was built and reverted
   in `97bb7b8`/`b0e8b0f` — recover it from history rather than rediscovering it.)
2. **Flashing tools still need the real part.** `boot/.cargo/config.toml` and
   `tools/flash-jlink.sh` pass `--chip STM32L471RGTx`; probe-rs uses it for flash
   algorithms and RAM layout. Point it at `STM32L476RGTx` for the foiling board.
   That is a tooling flag, not a property of the image.
3. This was checked against `embassy-stm32 v0.6.0` / `stm32-metapac 21.0.0`, and
   different silicon means different errata sheets. Nothing structural, but worth
   a look if something inexplicable turns up on the bench.

The one *hardware* difference is **pin 48: VDD on the L471, VDDUSB on the L476**.
The board ties it to 3v3, which is in range for both (`VDDUSB` is 3.0-3.6 V when
USB is used, 0-3.6 V when it is not). Because it is powered rather than grounded,
PA9-PA12 also keep full 5 V tolerance — which matters, because PA12 carries
`UART_DETECT4` on the display board.

## Building

```sh
# The three boards with a bootloader, flashed over CAN
./build-and-update-over-can.sh

# The foiling board: no bootloader, linked flat at 0x08000000, flashed over SWD
cargo build --release --bin foiling
```

`bootloader` is a **crate-wide** feature, so it cannot be on for some binaries
and off for others in one invocation. Consequences:

- Never `cargo build --bins --features bootloader`. It also produces a `foiling`
  image at the bootloader offset, which is wrong for that board and looks no
  different from a good one.
- The two builds share a target directory, so each overwrites the other's
  `foiling`. Rebuild it after running the OTA script.

Confirm what you flashed:

```sh
readelf -l target/thumbv7em-none-eabihf/release/foiling | awk '/LOAD/{print $3; exit}'
# 0x08000000 for foiling, 0x08014800 for the bootloader-hosted boards
```

## Display boards

The helm dashboard and the foiling display are the same board design running the
same decode pipeline. `display::run_display` owns the pin choices and the refresh
loop; each binary is a `bind_interrupts!`, an app type, and a
`draw_display::Layout`. Adding a screen should not mean touching either binary.

`bind_interrupts!` has to stay in the binaries — it emits `#[no_mangle]`
handlers, and one copy in the library would collide with the `CAN1_*` handlers
the rudder and height-sensor binaries declare for themselves.

Both layouts are linked into both images, because `Layout::draw` matches over the
enum. At ~99 KB of a 1 MB part that is not worth avoiding; if flash ever gets
tight, gate the layout behind a feature so LTO can drop the unused one.

## TODO: bootloader on the foiling board

Deferred deliberately. The board is flashed over SWD and nothing answers on its
bootloader address. To deploy CAN OTA on it later:

1. Add a `foil-tuning` feature to `boot` selecting `AppType::FoilTuning`, which
   already exists.
2. Add `FoilTuning` to the flash tool's `Board` enum — left out on purpose today
   so the CLI cannot offer a target that only times out.
3. Build the `foiling` binary with `--features bootloader`, which means splitting
   it out of the same invocation as the other three, or giving every board a
   bootloader.
4. Check `linker/boot.x`: `__app_end` is hardcoded to `0x080FF000`. Correct for
   1 MB, so an L476RG needs no change — but it *would* have for the 512 KB
   L471RE that was briefly the plan, so verify against the fitted part.

**Never let two boards on the bus report the same app type.** The app type *is*
the bootloader's CAN address, so a `REBOOT` or a flash aimed at one would hit
both, and the two would answer over each other.
