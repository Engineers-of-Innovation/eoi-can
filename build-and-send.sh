#!/bin/bash
# Build flash-tool for the Raspberry Pi (aarch64) and the firmware binaries,
# then send all three to the target host via scp.
#
# Prerequisites:
#   - cargo install cross   (cross uses Docker; Docker must be running)
#   - rustup target add thumbv7em-none-eabi
#
# Usage: ./build-and-send.sh <user@ip_address>

set -e

if [[ ! "$1" =~ ^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+$ ]]; then
    echo "Usage: $0 <user@ip_address>"
    exit 1
fi

host_arch="aarch64-unknown-linux-gnu" # RPi 3/4/5/Zero2 (64-bit Pi OS)
fw_arch="thumbv7em-none-eabi"

# Build flash-tool for the Pi (separate crate, excluded from workspace).
cd flash-tool
cross build --target ${host_arch} --release
cd ..

# Build firmware binaries (host cargo, default target = thumbv7em-none-eabi).
cargo build --release --bin rudder-controller        --features bootloader
cargo build --release --bin height-sensor-controller --features bootloader

# Send.
scp flash-tool/target/${host_arch}/release/eoi-flash-tool   ${1}:~/eoi-flash-tool
scp target/${fw_arch}/release/rudder-controller             ${1}:~/rudder-controller
scp target/${fw_arch}/release/height-sensor-controller      ${1}:~/height-sensor-controller
