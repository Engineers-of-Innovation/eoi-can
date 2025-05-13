#![no_main]

use eoi_can_decoder::can_frame::CanFrame;
use eoi_can_decoder::parse_eoi_can_data;
use libfuzzer_sys::fuzz_target;

// Test for checking if parse function does not panic for any input
fuzz_target!(|frame: CanFrame| {
    let _ = parse_eoi_can_data(&frame);
});
