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

# add second argument to compile for raspberry pi 4, e.g. ./build-and-send.sh engineer@10.12.0.208 rpi
if [[ $2 ]]; then
    arch="aarch64-unknown-linux-gnu" #RPI4
else
    arch="armv7-unknown-linux-gnueabihf" #old datalogger
fi

echo ${arch}
exit 1
# building
cd eoi-gnss-to-can
cross build --target ${arch} --release
cd ..

cd eoi-can-display-framebuffer
cross build --target ${arch} --release
cd ..

# sending, make sure you have you ssh keys set up in the datalogger, you might need to run ssh-copy-id

scp target/${arch}/release/eoi-can-display-framebuffer ${1}:~
scp target/${arch}/release/eoi-gnss-to-can ${1}:~
