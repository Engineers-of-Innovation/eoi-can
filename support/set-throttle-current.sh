#!/bin/bash
set -e

usage() {
    echo "Usage: $0 <amps> [can_iface]"
    echo "  amps:        max forward/backward current in Amps (0..3276)"
    echo "  can_iface:   CAN interface name (default: can0)"
    echo ""
    echo "Note: throttle must be OFF for the configuration to apply."
    exit 1
}

[[ $# -lt 1 || $# -gt 2 ]] && usage
amps="$1"
iface="${2:-can0}"

[[ "$amps" =~ ^[0-9]+$ ]] || { echo "Error: amps must be a non-negative integer"; exit 1; }
(( amps <= 3276 )) || { echo "Error: amps must be <= 3276 (int16_t limit in 100mA units)"; exit 1; }

units=$(( amps * 10 ))
hex=$(printf '%04X' "$units")
msg="AA03${hex}${hex}"

echo "Sending: cansend $iface 337#$msg  (current control, ${amps}A fwd/back)"
cansend "$iface" "337#$msg"
echo "Done. Remember: the throttle must be OFF for this to take effect."
