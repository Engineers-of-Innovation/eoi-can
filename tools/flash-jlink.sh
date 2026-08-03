#!/bin/bash
# Cargo runner for `cargo run` — wired up in .cargo/config.toml.
#
# By default this behaves exactly as the repo always has: it shells out to
# probe-rs with the same flags that used to live in .cargo/config.toml. The only
# platform that gets different treatment is WSL, where probe-rs cannot reach a
# USB probe; there the ELF is converted to hex and handed to the Windows-side
# J-Link Commander instead.
#
# Override the auto-detection with FLASH_BACKEND=probe-rs or FLASH_BACKEND=jlink
# (e.g. probe-rs on WSL with a usbipd-attached probe).
#
# Note: the J-Link path flashes and runs, but does not stream defmt/RTT logs the
# way probe-rs does.
set -e

backend="${FLASH_BACKEND:-}"
if [ -z "$backend" ]; then
    if grep -qi microsoft /proc/version 2>/dev/null; then
        backend=jlink
    else
        backend=probe-rs
    fi
fi

case "$backend" in
jlink)
    ELF="$1"
    for tool in arm-none-eabi-objcopy JLink.exe wslpath; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "flash-jlink.sh: '$tool' not found; needed for the jlink backend" >&2
            echo "  (set FLASH_BACKEND=probe-rs to use probe-rs instead)" >&2
            exit 1
        fi
    done

    HEX="${ELF}.hex"
    arm-none-eabi-objcopy -O ihex "$ELF" "$HEX"
    WIN_HEX=$(wslpath -w "$HEX")
    SCRIPT=$(mktemp /tmp/flash_XXXXXX.jlink)
    # Clean up even when JLink.exe fails and `set -e` aborts.
    trap 'rm -f "$SCRIPT"' EXIT
    WIN_SCRIPT=$(wslpath -w "$SCRIPT")

    cat > "$SCRIPT" << EOF
device STM32L471RG
si 1
speed 4000
loadfile "$WIN_HEX"
r
g
exit
EOF

    JLink.exe -commanderscript "$WIN_SCRIPT"
    ;;
probe-rs)
    if ! command -v probe-rs >/dev/null 2>&1; then
        echo "flash-jlink.sh: 'probe-rs' not found; install with 'cargo install probe-rs-tools'" >&2
        exit 1
    fi
    # "$@" rather than just the ELF, so trailing `cargo run -- <args>` still
    # reach probe-rs as they did when this was a plain runner array.
    probe-rs run \
        --chip STM32L471RGTx \
        --connect-under-reset \
        --catch-reset \
        --catch-hardfault \
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
