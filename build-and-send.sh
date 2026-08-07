#!/bin/bash
# Build flash-tool for the Raspberry Pi (aarch64) and the firmware binaries,
# then send them to the target host via scp.
#
# Prerequisites:
#   - cargo install cross   (cross uses Docker; Docker must be running)
#   - rustup target add thumbv7em-none-eabihf
#
# Usage: ./build-and-send.sh <user@ip_address> [board]
#
#   board  one of rudder-controller | height-sensor-controller | dashboard.
#          Omit to build and send all three.
#
# On the Pi, the flash tool picks its target from the ELF's app type, so
# `./eoi-flash-tool flash ~/dashboard` can only ever touch the dashboard.
# Use `./eoi-flash-tool scan` to see which boards are on the bus.

set -e

all_boards=(rudder-controller height-sensor-controller dashboard)

if [[ ! "$1" =~ ^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+$ ]]; then
    echo "Usage: $0 <user@ip_address> [${all_boards[*]}]"
    exit 1
fi

if [[ -n "$2" ]]; then
    # shellcheck disable=SC2076  # literal match is what we want here
    if [[ ! " ${all_boards[*]} " =~ " $2 " ]]; then
        echo "Unknown board '$2'. Expected one of: ${all_boards[*]}"
        exit 1
    fi
    boards=("$2")
else
    boards=("${all_boards[@]}")
fi

host_arch="aarch64-unknown-linux-gnu" # RPi 3/4/5/Zero2 (64-bit Pi OS)
fw_arch="thumbv7em-none-eabihf"

# Build flash-tool for the Pi (separate crate, excluded from workspace).
cd flash-tool
cross build --target ${host_arch} --release
cd ..

# Build firmware binaries (host cargo, default target = thumbv7em-none-eabihf).
for board in "${boards[@]}"; do
    cargo build --release --bin "${board}" --features bootloader
done

# Send.
scp flash-tool/target/${host_arch}/release/eoi-flash-tool "${1}:~/eoi-flash-tool"
for board in "${boards[@]}"; do
    scp "target/${fw_arch}/release/${board}" "${1}:~/${board}"
done
