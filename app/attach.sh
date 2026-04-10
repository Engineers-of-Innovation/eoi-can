#!/bin/bash
# Attach to a running application to view defmt RTT logs without flashing.
# Usage: ./attach.sh <binary>
# Example: ./attach.sh rudder-controller
#          ./attach.sh height-sensor-controller

set -e

BIN="${1:?Usage: $0 <binary-name>}"
ELF="../target/thumbv7em-none-eabi/release/$BIN"

if [ ! -f "$ELF" ]; then
    echo "ELF not found: $ELF"
    echo "Build it first: cargo build --release --bin $BIN"
    exit 1
fi

probe-rs attach \
    --chip STM32L471RGTx \
    --log-format "{L} {t} {f}:{l} {s}" \
    "$ELF"
