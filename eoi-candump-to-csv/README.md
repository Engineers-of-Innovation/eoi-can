# eoi-candump-to-csv

Converts a candump `.log` capture into a CSV by decoding frames through
[`eoi-can-decoder`](../eoi-can-decoder) and sampling at a chosen interval.
Same decoding as the live MQTT bridge, so anything `eoi-can-to-mqtt`
understands shows up as a CSV column.

## Quick start

```sh
# Default: 1 s buckets, output beside the input as <input>.csv
cargo run -p eoi-candump-to-csv -- candump-2026-05-15_094333.log -d battery,gnss

# Every decoded MPPT-1 frame, custom output path
cargo run -p eoi-candump-to-csv -- some.log -d mppt:1 --interval frame -o mppt1.csv

# Everything, 100 ms buckets
cargo run -p eoi-candump-to-csv -- some.log -d all --interval 100ms
```

See `eoi-candump-to-csv --help` for the full flag list and the set of
device selectors.

The CAN ID → signal mapping itself lives in
[CAN_MESSAGES.md](../CAN_MESSAGES.md) and
[eoi-can-decoder/src/lib.rs](../eoi-can-decoder/src/lib.rs).
