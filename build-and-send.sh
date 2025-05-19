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

# building
cd eoi-gnss-to-can
cross build --target armv7-unknown-linux-gnueabihf --release
cd ..

cd eoi-can-display-framebuffer
cross build --target armv7-unknown-linux-gnueabihf --release
cd ..

# sending, make sure you have you ssh keys set up in the datalogger, you might need to run ssh-copy-id

scp target/armv7-unknown-linux-gnueabihf/release/eoi-can-display-framebuffer ${1}:~
scp target/armv7-unknown-linux-gnueabihf/release/eoi-gnss-to-can ${1}:~