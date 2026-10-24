#!/bin/bash
# Build the app firmware and the flash tool, then flash the boards directly
# over the local CAN interface.
#
# Prerequisites:
#   - rustup target add thumbv7em-none-eabihf
#   - CAN interface up, e.g.:
#       sudo ip link set can0 type can bitrate 1000000
#       sudo ip link set can0 up
#
# Usage: ./build-and-update-over-can.sh [-i <iface>] [board]
#
#   -i     CAN interface to use (default: can0).
#   board  one of rudder-controller | height-sensor-controller | dashboard |
#          foiling. Omit to build and flash all four.
#
# The flash tool picks its target from the ELF's app type, so each ELF can
# only ever end up on its own board. Boards that fail to flash are reported
# at the end; the script keeps going so the remaining boards still update.
#
# A write is retried once. The bus is shared with a live boat, and a dropped
# write ack shows up as `Timeout waiting for response` part way through -- seen
# at 27% on the foiling board, with an identical retry succeeding. The retry is
# safe and needs no recovery step: an interrupted board sits in `FlashingApp`,
# which the flash tool counts as a bootloader state, so the next attempt goes
# straight back to erase and resets the write pointer. The bootloader lives in
# the first 80K and is never touched by this path.
#
# Every board, the foiling display included, runs the CAN bootloader, so every
# image is built with `--features bootloader` and linked at 0x08014800. A plain
# `cargo build --bin foiling` (no feature) links flat at 0x08000000; flashing
# that over SWD overwrites the bootloader and takes the board off CAN OTA. To
# put the bootloader back over SWD, `cargo run --release --features bootloader
# --bin foiling` -- the runner flashes bootloader, header and app together.

set -e

all_boards=(rudder-controller height-sensor-controller dashboard foiling)
iface="can0"

usage() {
    echo "Usage: $0 [-i <iface>] [${all_boards[*]}]"
    exit 1
}

while getopts "i:" opt; do
    case "$opt" in
        i) iface="$OPTARG" ;;
        *) usage ;;
    esac
done
shift $((OPTIND - 1))

if [[ -n "$1" ]]; then
    # shellcheck disable=SC2076  # literal match is what we want here
    if [[ ! " ${all_boards[*]} " =~ " $1 " ]]; then
        echo "Unknown board '$1'. Expected one of: ${all_boards[*]}"
        usage
    fi
    boards=("$1")
else
    boards=("${all_boards[@]}")
fi

fw_arch="thumbv7em-none-eabihf"
host_arch="x86_64-unknown-linux-gnu"
flash_tool="flash-tool/target/${host_arch}/release/eoi-flash-tool"

cd "$(dirname "$0")"

# `built` stamps the commit and a dirty flag into every image, and the flag is
# repo-wide: any modified tracked file anywhere in the checkout sets it, not
# just a firmware one. The boards then report `git <hash>-dirty` forever, which
# is no longer traceable to a commit. Warn rather than refuse -- flashing a work
# in progress to the bench is normal -- but say so before the build, not after.
if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
    echo "WARNING: working tree is dirty; images will report 'git <hash>-dirty'."
    echo "         Commit or stash first for a traceable build."
    echo
fi

# Build flash-tool (separate crate, excluded from workspace).
(cd flash-tool && cargo build --release)

# Build firmware binaries (host cargo, default target = thumbv7em-none-eabihf).
# Boards are named explicitly rather than with `--bins`, so the script flashes
# only what it was asked to.
bin_args=()
for board in "${boards[@]}"; do
    bin_args+=(--bin "${board}")
done
cargo build --release -p eoi-firmware --features bootloader "${bin_args[@]}"

# Flash each board over CAN; keep going if one fails.
failed=()
for board in "${boards[@]}"; do
    for attempt in 1 2; do
        echo "==> Flashing ${board} on ${iface} (attempt ${attempt})"
        if "${flash_tool}" -i "${iface}" flash "target/${fw_arch}/release/${board}"; then
            break
        fi
        if [[ ${attempt} -eq 2 ]]; then
            failed+=("${board}")
        else
            echo "    write failed; retrying ${board}"
        fi
    done
done

echo
echo "Summary:"
for board in "${boards[@]}"; do
    # shellcheck disable=SC2076  # literal match is what we want here
    if [[ " ${failed[*]} " =~ " ${board} " ]]; then
        echo "  ${board}: FAILED"
    else
        echo "  ${board}: OK"
    fi
done

if [[ ${#failed[@]} -gt 0 ]]; then
    exit 1
fi
