# EoI CAN Project

This repository contains a collection of tools and firmware for working with our (Engineers of Innovation) CAN (Controller Area Network) devices. It includes Rust-based applications, firmware, utilities, and support scripts for CAN data collection, decoding, display, and integration with other systems (like MQTT and GNSS).

## Table of Contents

- [EoI CAN Project](#eoi-can-project)
  - [Table of Contents](#table-of-contents)
  - [Overview](#overview)
  - [Project Structure](#project-structure)
  - [Getting Started](#getting-started)
    - [Prerequisites](#prerequisites)
    - [Clone the Repository](#clone-the-repository)
  - [Building the Project](#building-the-project)
  - [Running the Tools](#running-the-tools)
  - [Build and send script](#build-and-send-script)
  - [Support Scripts \& Services](#support-scripts--services)
  - [Contributing](#contributing)
  - [License](#license)

## Overview

This repository is designed to help you:

- Collect and decode CAN bus data
- Display CAN data on embedded devices
- Simulate CAN display output
- Bridge CAN data to MQTT
- Integrate GNSS data with CAN
- Manage device power and services

You do **not** need to be a Rust expert to get started! This guide will help you build and run the tools, and provides pointers for further development.

## Project Structure

- `draw-display/` — Library for drawing on display devices
  - Used in all `eoi-can-display-*` projects
  - Original designed for an black and white e-ink display
- `eoi-can-decoder/` — CAN data decoding utilities
  - Made in a way so it can be used for displaying data but also can easily be converted to JSON (to be send over MQTT)
- `eoi-can-display-firmware/` — Firmware for the CAN display
  - Connects to a eink display with our `RS485 to CAN` board
- `eoi-can-display-framebuffer/` — Framebuffer-based display application
  - Can be run on a linux machine with a standard Raspberry Pi display (800x480 pixels)
- `eoi-can-display-simulator/` — Simulator for the CAN display
  - Just runs on your computer, you only need to connect a CAN bus
- `eoi-can-to-mqtt/` — Bridge for sending CAN data to MQTT
  - Collects CAN messages and decodes and sends it over to our MQTT broker
- `eoi-gnss-to-can/` — GNSS to CAN integration
  - A simple program to send GNSS/GPS information on the CAN bus, since this way we only need to log the CAN bus
- `get-wifi-ip/` — Crate for getting WiFi IP address
- `pisugar/` — Crate for getting PiSugar's battery information
- `support/` — Shell scripts and systemd service files running on the data logger in the boat
- `fuzz/` — Fuzz testing for CAN decoder

## Getting Started

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (install with `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- [cargo](https://doc.rust-lang.org/cargo/) (comes with Rust)
- For some tools: [systemd](https://www.freedesktop.org/wiki/Software/systemd/) (for running services), [mosquitto](https://mosquitto.org/) (for MQTT), and other dependencies as needed

### Clone the Repository

```sh
git clone https://git.engineersofinnovation.nl/boat-fw/eoi-can
cd eoi-can
```

## Building the Project

To build all Rust projects:

```sh
cargo build
```

This will compile all binaries in the workspace. You can also build a specific tool, e.g.:

```sh
cd eoi-can-to-mqtt
cargo build
```

## Running the Tools

Each subdirectory contains a Rust project or utility. For example, to run the CAN-to-Display on can0:

```sh
cd eoi-can-display-simulator
cargo run -- -ccan0
```

For firmware or embedded targets, see the specific subproject's README or source for details on flashing or running on hardware.

## Build and send script

The `build-and-send.sh` script can be used to compile and send the related files to the data logger. See the script itself how to use it.

## Support Scripts & Services

The `support/` directory contains useful shell scripts and systemd service files for automating startup, power management, and monitoring on the data logger. For example:

- `install.sh` — Installs systemd services
- `status.sh` — Checks service status
- `auto-poweroff.sh` — Power management script

To install services (run this on the target, not your computer):

```sh
cd support
sudo ./install.sh
```

## Contributing

Contributions are welcome! Please open issues or pull requests. If you are new to Rust, feel free to ask questions or suggest improvements to documentation.

## License

This project is licensed under the MIT License. See `LICENSE` for details.
