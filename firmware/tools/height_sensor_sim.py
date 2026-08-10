#!/usr/bin/env python3
"""
Simulates the Modbus RTU height sensor on RS485.

Responds to: slave 0x01, read holding registers (FC 0x03), register 0x0101.
UART config: 9600 baud, 8N2.

Usage:
    python height_sensor_sim.py /dev/ttyUSB0
    python height_sensor_sim.py /dev/ttyUSB0 --height 1234
"""

import argparse
import struct
import sys
import time

import serial


def crc16_modbus(data: bytes) -> int:
    crc = 0xFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            if crc & 1:
                crc = (crc >> 1) ^ 0xA001
            else:
                crc >>= 1
    return crc


def build_response(slave: int, func: int, registers: list[int]) -> bytes:
    payload = struct.pack("BBB", slave, func, len(registers) * 2)
    for reg in registers:
        payload += struct.pack(">H", reg)
    crc = crc16_modbus(payload)
    return payload + struct.pack("<H", crc)


def build_error(slave: int, func: int, exception_code: int) -> bytes:
    payload = struct.pack("BBB", slave, func | 0x80, exception_code)
    crc = crc16_modbus(payload)
    return payload + struct.pack("<H", crc)


def main():
    parser = argparse.ArgumentParser(description="Height sensor Modbus RTU simulator")
    parser.add_argument(
        "port",
        nargs="?",
        default="/dev/ttyUSB0",
        help="Serial port (default: /dev/ttyUSB0)",
    )
    parser.add_argument("--height", type=int, default=500, help="Height value in mm (default: 500)")
    parser.add_argument("--sweep", action="store_true", help="Sweep height 0-1000mm in a sine wave")
    args = parser.parse_args()

    ser = serial.Serial(
        port=args.port,
        baudrate=9600,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_TWO,
        timeout=0.1,
    )
    print(f"Listening on {args.port} as Modbus slave 0x01 (height={args.height} mm, sweep={args.sweep})")

    height = args.height
    t0 = time.monotonic()
    buf = bytearray()

    while True:
        chunk = ser.read(256)
        if chunk:
            buf.extend(chunk)
        else:
            # Inter-frame silence — process if we have data
            if len(buf) < 8:
                if buf:
                    buf.clear()
                continue

            # Minimum Modbus RTU request: slave(1) + func(1) + start(2) + count(2) + crc(2) = 8
            slave = buf[0]
            func = buf[1]
            crc_recv = struct.unpack_from("<H", buf, len(buf) - 2)[0]
            crc_calc = crc16_modbus(bytes(buf[:-2]))

            if crc_recv != crc_calc:
                print(f"  CRC mismatch (got 0x{crc_recv:04X}, expected 0x{crc_calc:04X}), ignoring")
                buf.clear()
                continue

            if slave != 0x01:
                print(f"  Not for us (slave 0x{slave:02X}), ignoring")
                buf.clear()
                continue

            print(f"<< RX [{len(buf)}]: {buf.hex(' ')}")

            if func == 0x03:  # Read Holding Registers
                start_reg = struct.unpack_from(">H", buf, 2)[0]
                count = struct.unpack_from(">H", buf, 4)[0]
                print(f"  FC03: start=0x{start_reg:04X} count={count}")

                if start_reg == 0x0101 and count == 1:
                    if args.sweep:
                        import math
                        height = int(500 + 500 * math.sin((time.monotonic() - t0) * 0.5))
                    resp = build_response(0x01, 0x03, [height])
                    print(f">> TX [{len(resp)}]: {resp.hex(' ')} (height={height} mm)")
                    ser.write(resp)
                else:
                    resp = build_error(0x01, 0x03, 0x02)  # Illegal Data Address
                    print(f">> TX error: illegal address")
                    ser.write(resp)
            else:
                resp = build_error(0x01, func, 0x01)  # Illegal Function
                print(f">> TX error: illegal function 0x{func:02X}")
                ser.write(resp)

            buf.clear()


if __name__ == "__main__":
    main()
