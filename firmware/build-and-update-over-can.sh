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
#   board  one of rudder-controller | height-sensor-controller | dashboard.
#          Omit to build and flash all three.
#
# The flash tool picks its target from the ELF's app type, so each ELF can
# only ever end up on its own board. Boards that fail to flash are reported
# at the end; the script keeps going so the remaining boards still update.

set -e

all_boards=(rudder-controller height-sensor-controller dashboard)
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

# Build flash-tool (separate crate, excluded from workspace).
(cd flash-tool && cargo build --release)

# Build firmware binaries (host cargo, default target = thumbv7em-none-eabihf).
if [[ ${#boards[@]} -eq 1 ]]; then
    cargo build --release --bin "${boards[0]}" --features bootloader
else
    cargo build --release --bins --features bootloader
fi

# Flash each board over CAN; keep going if one fails.
failed=()
for board in "${boards[@]}"; do
    echo "==> Flashing ${board} on ${iface}"
    if ! "${flash_tool}" -i "${iface}" flash "target/${fw_arch}/release/${board}"; then
        failed+=("${board}")
    fi
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
