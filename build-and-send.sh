#!/bin/bash
# This script builds the eoi-gnss-to-can and eoi-can-display-framebuffer projects for the datalogger
# and sends the binaries to the datalogger.
# Make sure you have cross and cargo installed (cargo install cross)
#
# argument is the ip address of the datalogger
# usage: ./build-and-send-to-datalogger.sh <ip_address>

# check if user@address is provided, e.g root@192.168.0.1 or foo@bar.com
if [[ ! "$1" =~ ^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+$ ]]; then
    echo "Usage: $0 <user@ip_address>"
    exit 1
fi

if ! command -v cross &> /dev/null; then
    echo "cross is not installed. Install it with: cargo install cross"
    exit 1
fi

# cross builds inside a container; without a running container engine it falls
# back to a local rustup toolchain install, which fails with an unrelated-looking
# "couldn't install toolchain" error. Catch the real cause here instead.
if command -v docker &> /dev/null && docker info &> /dev/null; then
    :
elif command -v podman &> /dev/null && podman info &> /dev/null; then
    :
else
    echo "No running container engine found (Docker or Podman)."
    echo "cross needs one to cross-compile; start Docker Desktop (or Podman) and try again."
    exit 1
fi

# cross's images are only published for linux/amd64; on Apple Silicon, Docker
# needs to be told to run them under emulation rather than look for a native
# arm64 image (which doesn't exist and fails with "no matching manifest").
export DOCKER_DEFAULT_PLATFORM=linux/amd64

arch="aarch64-unknown-linux-gnu" #RPI4

# building
cd eoi-gnss-to-can
cross build --target ${arch} --release
cd ..

cd eoi-can-display-framebuffer
cross build --target ${arch} --release
cd ..

cd eoi-can-to-mqtt
cross build --target ${arch} --release
cd ..

cd eoi-can-to-grpc
cross build --target ${arch} --release
cd ..

# # sending, make sure you have you ssh keys set up in the datalogger, you might need to run ssh-copy-id

scp target/${arch}/release/eoi-can-display-framebuffer ${1}:~/eoi-can-display-framebuffer.new
scp target/${arch}/release/eoi-gnss-to-can ${1}:~/eoi-gnss-to-can.new
scp target/${arch}/release/eoi-can-to-mqtt ${1}:~/eoi-can-to-mqtt.new
scp target/${arch}/release/eoi-can-to-grpc ${1}:~/eoi-can-to-grpc.new
scp -r support ${1}:~
