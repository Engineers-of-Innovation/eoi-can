#!/bin/bash
# One-time base setup for a freshly flashed datalogger SD card. Run on the Pi,
# after build-and-send.sh has copied this directory to ~/support:
#
#   sudo ~/support/setup-sd.sh
#   sudo reboot
#   sudo ~/support/install.sh
#
# Safe to run again: config.txt is only appended to once.
#
# Hardware this assumes (change here if the boat changes):
#   - Raspberry Pi 4, Raspberry Pi OS Lite 64-bit, user "engineer"
#   - skpang PiCAN Duo: two MCP2515s on SPI0 with a 16 MHz crystal, boat bus on CAN0
#   - Official 7" DSI touchscreen, drawn on through /dev/fb0
#   - PiSugar 3, read over TCP on 127.0.0.1:8423 (auto-poweroff.sh, pisugar crate)
#   - A Bluetooth keyboard for the foil tuner (pair it once with bluetoothctl)

set -e

if [[ $EUID -ne 0 ]]; then
    echo "Run as root: sudo $0"
    exit 1
fi

USER_NAME="engineer"
USER_DIR="/home/${USER_NAME}"
SUPPORT_DIR="${USER_DIR}/support"
SYSTEMD_DIR="/etc/systemd/system"
MARKER="# --- eoi-can datalogger (setup-sd.sh) ---"

if ! id "$USER_NAME" &> /dev/null; then
    echo "User $USER_NAME does not exist. Create it in Raspberry Pi Imager's settings and reflash."
    exit 1
fi

# Bookworm and later moved the boot partition to /boot/firmware.
if [[ -f /boot/firmware/config.txt ]]; then
    CONFIG_TXT=/boot/firmware/config.txt
else
    CONFIG_TXT=/boot/config.txt
fi

echo "Installing packages..."
apt-get update
# can-utils: candump for eoi-can-logger, cansend for set-throttle-current.sh
# netcat-openbsd: auto-poweroff.sh asks pisugar-server for the power state
# python3-can, python3-evdev: the keyboard tuner, tools/foil_tune.py in the
#   ardupilot repo, copied to ~/tools by hand
apt-get install -y can-utils netcat-openbsd i2c-tools curl python3-can python3-evdev

echo "Enabling SPI, I2C and the CAN controllers in $CONFIG_TXT..."
if grep -qF "$MARKER" "$CONFIG_TXT"; then
    echo "  -> Already done, skipping"
else
    # Only the PiCAN Duo's CAN0 port (SPI CE0) is enabled. With both controllers
    # loaded the kernel names them in probe order, so the boat's bus can come up
    # as can1 and every service listening on can0 hears nothing. For the CAN1
    # port add: dtoverlay=mcp2515-can1,oscillator=16000000,interrupt=24
    #
    # The 7" DSI panel is named outright: display_auto_detect did not find it.
    cat >> "$CONFIG_TXT" << EOF

$MARKER
[all]
dtparam=spi=on
dtparam=i2c_arm=on
dtoverlay=mcp2515-can0,oscillator=16000000,interrupt=25
dtoverlay=vc4-kms-dsi-7inch
EOF
fi

CMDLINE_TXT="$(dirname "$CONFIG_TXT")/cmdline.txt"
echo "Setting a 32-bit framebuffer on the DSI panel in $CMDLINE_TXT..."
# KMS hands out a 16-bit /dev/fb0 by default. eoi-foil-view packs any 16/24/32-bit
# layout, but 16-bit bands its colour gradients and anti-aliasing.
if grep -q "video=DSI-1" "$CMDLINE_TXT"; then
    echo "  -> Already done, skipping"
else
    sed -i '1 s/$/ video=DSI-1:800x480-32@60/' "$CMDLINE_TXT"
fi

echo "Creating the candump log directory..."
install -d -o "$USER_NAME" -g "$USER_NAME" "${USER_DIR}/logs"

echo "Giving $USER_NAME access to the framebuffer..."
usermod -aG video "$USER_NAME"

# install.sh skips this unit, so a new card has to get it here. Every other
# eoi- service Requires= it.
echo "Installing eoi-can-init.service..."
cp "${SUPPORT_DIR}/eoi-can-init.service" "$SYSTEMD_DIR/"
systemctl daemon-reload
systemctl enable eoi-can-init.service

if systemctl cat pisugar-server.service &> /dev/null; then
    echo "pisugar-server already installed, skipping"
else
    echo "Installing pisugar-server..."
    # Answer the packages' model questions up front so this runs over plain ssh.
    debconf-set-selections << EOF
pisugar-server pisugar-server/model select PiSugar 3
pisugar-poweroff pisugar-poweroff/model select PiSugar 3
EOF
    curl -fsSL https://cdn.pisugar.com/release/pisugar-power-manager.sh -o /tmp/pisugar-power-manager.sh
    DEBIAN_FRONTEND=noninteractive bash /tmp/pisugar-power-manager.sh -c release
fi

if [[ ! -f "${USER_DIR}/tools/foil_tune.py" ]]; then
    echo
    echo "Still to do by hand: the foil tuner, which lives in the ardupilot repo"
    echo "  scp <ardupilot>/tools/foil_tune.py engineer@<pi>:tools/   # eoi-foil-tune.service runs it"
fi

if [[ ! -f "${USER_DIR}/eoi-can-to-mqtt.env" ]]; then
    echo
    echo "Still to do by hand: the MQTT password"
    echo "  cp ${SUPPORT_DIR}/eoi-can-to-mqtt.env.example ${USER_DIR}/eoi-can-to-mqtt.env"
    echo "  chmod 600 ${USER_DIR}/eoi-can-to-mqtt.env   # then set MQTT_PASSWORD"
fi

echo
echo "Done. Next: sudo reboot, then sudo ${SUPPORT_DIR}/install.sh"
echo "After the reboot, 'ip -details link show can0' should list an mcp251x device."
