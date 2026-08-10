#!/bin/bash
# Cargo runner for `cargo run` — wired up in .cargo/config.toml.
#
# It does two things a bare `probe-rs run` does not:
#
#   Backend choice. probe-rs when a probe is reachable, which now includes WSL
#   with the probe attached via `usbipd attach --wsl`. On WSL without an
#   attached probe it falls back to converting the ELF to hex and handing it to
#   the Windows-side J-Link Commander (flashes and runs, but cannot stream
#   defmt/RTT logs). Override with FLASH_BACKEND=probe-rs or FLASH_BACKEND=jlink.
#
#   Bootloader awareness. An app built with --features bootloader is linked at
#   0x08014800 and cannot boot on its own: the reset vector at 0x08000000
#   belongs to the bootloader, which only starts an app that a valid header at
#   0x08014000 vouches for. For such an ELF this script also builds the
#   matching bootloader variant, generates the header, and flashes all three,
#   so one `cargo run` leaves a device that actually comes up. Apps built
#   without the feature own all of flash and are flashed alone, as before.
#   Set FLASH_BOOTLOADER=0 to flash just the app when the bootloader is already
#   on the device.
set -e

ELF="$1"
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

CHIP=STM32L471RGTx        # probe-rs chip name
JLINK_DEVICE=STM32L471RG  # J-Link Commander device name
HEADER_BASE=0x08014000    # HEADER partition, see linker/app.x
APP_BASE=0x08014800       # FLASH origin with --features bootloader

# nRESET is not routed to the debug connector on these boards, so probe-rs has
# to attach to a running core; asking it to attach under reset times out. Set
# FLASH_CONNECT_UNDER_RESET=1 for a board where the pin is wired.
ATTACH=()
if [ "${FLASH_CONNECT_UNDER_RESET:-0}" != "0" ]; then
    ATTACH=(--connect-under-reset)
fi

backend="${FLASH_BACKEND:-}"
if [ -z "$backend" ]; then
    if command -v probe-rs >/dev/null 2>&1 && probe-rs list 2>/dev/null | grep -q '^\['; then
        backend=probe-rs
    elif grep -qi microsoft /proc/version 2>/dev/null; then
        # WSL with no probe attached to Linux — the probe is (still) on Windows.
        backend=jlink
    else
        backend=probe-rs
    fi
fi

# The lowest load address tells the two linker scripts apart: linker/app.x
# (--features bootloader) starts the app at APP_BASE, linker/app-dev.x owns
# flash from 0x08000000 and needs no bootloader underneath it.
lowest_lma=$(arm-none-eabi-readelf -lW "$ELF" | awk '$1 == "LOAD" { print $4 }' | sort | head -1)

BOOT_ELF=""
APP_IMAGE=""
if [ "$lowest_lma" = "$APP_BASE" ] && [ "${FLASH_BOOTLOADER:-1}" != "0" ]; then
    BIN=$(basename "$ELF")
    case "$BIN" in
    rudder-controller | height-sensor-controller) ;;
    *)
        echo "flash-jlink.sh: '$BIN' is linked at $APP_BASE but has no matching" >&2
        echo "  bootloader variant; set FLASH_BOOTLOADER=0 to flash it alone" >&2
        exit 1
        ;;
    esac

    # The bootloader is compiled for one app type and refuses to boot the other,
    # so the variant has to follow the binary being flashed. Its feature names
    # are deliberately identical to the app's [[bin]] names.
    echo "flash-jlink.sh: building bootloader for $BIN"
    cargo build --release --manifest-path "$REPO_ROOT/boot/Cargo.toml" --features "$BIN"
    BOOT_ELF="$REPO_ROOT/target/thumbv7em-none-eabihf/release/eoi-boot"

    # Header and app are generated as one blob so the CRC in the header always
    # describes the bytes flashed next to it. The flash tool already knows how
    # to derive both from the ELF, so don't reimplement the layout here.
    FLASH_TOOL="$REPO_ROOT/flash-tool/target/x86_64-unknown-linux-gnu/release/eoi-flash-tool"
    if [ ! -x "$FLASH_TOOL" ]; then
        echo "flash-jlink.sh: building the flash tool (generates the app header)"
        cargo build --release --manifest-path "$REPO_ROOT/flash-tool/Cargo.toml"
    fi
    APP_IMAGE="${ELF}.img"
    "$FLASH_TOOL" image "$ELF" --output "$APP_IMAGE"
fi

case "$backend" in
jlink)
    for tool in arm-none-eabi-objcopy JLink.exe wslpath; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "flash-jlink.sh: '$tool' not found; needed for the jlink backend" >&2
            echo "  (set FLASH_BACKEND=probe-rs to use probe-rs instead)" >&2
            exit 1
        fi
    done

    SCRIPT=$(mktemp /tmp/flash_XXXXXX.jlink)
    # Clean up even when JLink.exe fails and `set -e` aborts.
    trap 'rm -f "$SCRIPT"' EXIT
    WIN_SCRIPT=$(wslpath -w "$SCRIPT")

    {
        echo "device $JLINK_DEVICE"
        echo "si 1"
        echo "speed 4000"
        if [ -n "$BOOT_ELF" ]; then
            BOOT_HEX="${BOOT_ELF}.hex"
            arm-none-eabi-objcopy -O ihex "$BOOT_ELF" "$BOOT_HEX"
            echo "loadfile \"$(wslpath -w "$BOOT_HEX")\""
            # Raw binary, so it needs its destination spelled out.
            echo "loadbin \"$(wslpath -w "$APP_IMAGE")\", $HEADER_BASE"
        else
            HEX="${ELF}.hex"
            arm-none-eabi-objcopy -O ihex "$ELF" "$HEX"
            echo "loadfile \"$(wslpath -w "$HEX")\""
        fi
        echo "r"
        echo "g"
        echo "exit"
    } > "$SCRIPT"

    JLink.exe -commanderscript "$WIN_SCRIPT"
    ;;
probe-rs)
    if ! command -v probe-rs >/dev/null 2>&1; then
        echo "flash-jlink.sh: 'probe-rs' not found; install with 'cargo install probe-rs-tools'" >&2
        exit 1
    fi

    if [ -n "$BOOT_ELF" ]; then
        echo "flash-jlink.sh: flashing bootloader"
        probe-rs download \
            --chip "$CHIP" \
            "${ATTACH[@]}" \
            --preverify \
            --verify \
            "$BOOT_ELF"
        echo "flash-jlink.sh: flashing app header"
        probe-rs download \
            --chip "$CHIP" \
            "${ATTACH[@]}" \
            --preverify \
            --verify \
            --binary-format bin \
            --base-address "$HEADER_BASE" \
            "$APP_IMAGE"
    fi

    # "$@" rather than just the ELF, so trailing `cargo run -- <args>` still
    # reach probe-rs as they did when this was a plain runner array.
    # The reset below enters the bootloader, which hands over to the app; its
    # defmt logs decode against the app ELF, so the bootloader's own lines may
    # come out garbled until the handover.
    probe-rs run \
        --chip "$CHIP" \
        "${ATTACH[@]}" \
        --always-print-stacktrace \
        --log-format '{L} {t} {f}:{l} {s}' \
        --preverify \
        --verify \
        "$@"
    ;;
*)
    echo "flash-jlink.sh: unknown FLASH_BACKEND '$backend' (expected 'probe-rs' or 'jlink')" >&2
    exit 1
    ;;
esac
