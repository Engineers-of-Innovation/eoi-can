//! The firmware header is written by the flash tool and read by the bootloader,
//! and the bootloader can only be replaced over SWD. If the two ever disagree
//! about a byte offset, a CRC algorithm or a validation code, every OTA update
//! bricks a board with no way back short of a debug probe. These tests pin the
//! format down from both sides.

use eoi_boot_api::header::AppType;
use eoi_boot_api::header::{
    HEADER_MAGIC, HEADER_PARTITION_SIZE, HEADER_VERSION, HeaderError, HeaderInfo, ValidationResult,
    build_header, compute_crc,
};

/// Read `app` back through the closure shape `validate` expects.
fn reader(app: &[u8]) -> impl FnMut(&mut [u8]) -> Result<usize, ()> + '_ {
    let mut offset = 0usize;
    move |buf: &mut [u8]| {
        let n = buf.len().min(app.len() - offset);
        buf[..n].copy_from_slice(&app[offset..offset + n]);
        offset += n;
        Ok(n)
    }
}

fn header_for(app: &[u8], app_type: AppType) -> [u8; HEADER_PARTITION_SIZE] {
    build_header(app.len() as u32, compute_crc(app), app_type)
}

/// The exact layout the README documents. Written as raw indices rather than
/// through the accessors, so a reshuffle of the struct cannot quietly move a
/// field out from under an already-flashed bootloader.
#[test]
fn build_header_matches_the_documented_byte_layout() {
    let h = build_header(0x1234_5678, 0x9ABC_DEF0, AppType::Dashboard);
    assert_eq!(&h[0x00..0x04], &HEADER_MAGIC, "magic at 0x00");
    assert_eq!(h[0x04], HEADER_VERSION, "header version at 0x04");
    assert_eq!(&h[0x05..0x09], &0x1234_5678u32.to_le_bytes(), "len at 0x05");
    assert_eq!(&h[0x09..0x0D], &0x9ABC_DEF0u32.to_le_bytes(), "crc at 0x09");
    assert_eq!(h[0x0D], AppType::Dashboard as u8, "app type at 0x0D");
    assert_eq!(h.len(), 2048);
    // Padding must be 0xFF: that is the erased state of STM32L4 flash, so a
    // freshly erased page reads as a well-formed (if empty) tail.
    assert!(h[0x0E..].iter().all(|&b| b == 0xFF), "padding is not 0xFF");
}

#[test]
fn a_header_written_by_the_flash_tool_parses_in_the_bootloader() {
    for app_type in [
        AppType::RudderController,
        AppType::HeightSensorController,
        AppType::Dashboard,
    ] {
        let app: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let parsed = HeaderInfo::from_bytes(&header_for(&app, app_type)).expect("must parse");
        assert_eq!(parsed.app_len, app.len() as u32);
        assert_eq!(parsed.app_crc, compute_crc(&app));
        assert_eq!(parsed.app_type, app_type);
        parsed
            .validate(1_000_000, &mut reader(&app))
            .expect("a freshly built header must validate against its own app");
    }
}

#[test]
fn erased_flash_is_rejected_rather_than_booted() {
    // An erased header partition is all 0xFF — the state after `erase`, and
    // what a board with no application has.
    let erased = [0xFF_u8; HEADER_PARTITION_SIZE];
    assert!(matches!(
        HeaderInfo::from_bytes(&erased),
        Err(HeaderError::BadMagic)
    ));
}

#[test]
fn from_bytes_rejects_every_kind_of_malformed_header() {
    let app = [0xAB_u8; 64];
    let good = header_for(&app, AppType::Dashboard);

    let mut bad_magic = good;
    bad_magic[0] ^= 0xFF;
    assert!(matches!(
        HeaderInfo::from_bytes(&bad_magic),
        Err(HeaderError::BadMagic)
    ));

    // A future header format must be refused, not misread with these offsets.
    let mut bad_version = good;
    bad_version[0x04] = HEADER_VERSION + 1;
    assert!(matches!(
        HeaderInfo::from_bytes(&bad_version),
        Err(HeaderError::BadVersion)
    ));

    let mut zero_len = good;
    zero_len[0x05..0x09].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(
        HeaderInfo::from_bytes(&zero_len),
        Err(HeaderError::ZeroLength)
    ));

    // 0x00 and 0x04 bracket the valid app types; neither may parse.
    for byte in [0x00, 0x04, 0xFF] {
        let mut bad_type = good;
        bad_type[0x0D] = byte;
        assert!(
            matches!(
                HeaderInfo::from_bytes(&bad_type),
                Err(HeaderError::BadAppType)
            ),
            "app type 0x{byte:02X} must not parse"
        );
    }
}

#[test]
fn validate_rejects_an_app_that_does_not_fit_the_partition() {
    let app = [0xAB_u8; 64];
    let parsed = HeaderInfo::from_bytes(&header_for(&app, AppType::Dashboard)).unwrap();
    assert!(matches!(
        parsed.validate(app.len() as u32 - 1, &mut reader(&app)),
        Err(HeaderError::AppTooLarge)
    ));
    // Exactly filling the partition is allowed.
    parsed
        .validate(app.len() as u32, &mut reader(&app))
        .expect("an app that exactly fits must validate");
}

#[test]
fn validate_catches_a_corrupted_or_truncated_app() {
    let app: Vec<u8> = (0..512u32).map(|i| i as u8).collect();
    let parsed = HeaderInfo::from_bytes(&header_for(&app, AppType::Dashboard)).unwrap();

    // One flipped bit anywhere in the image must fail the CRC.
    let mut corrupt = app.clone();
    corrupt[300] ^= 0x01;
    assert!(matches!(
        parsed.validate(1_000_000, &mut reader(&corrupt)),
        Err(HeaderError::BadAppCrc)
    ));

    // A short read (interrupted write) must not pass either.
    assert!(matches!(
        parsed.validate(1_000_000, &mut reader(&app[..256])),
        Err(HeaderError::BadAppCrc)
    ));

    // A flash read that errors out is a failure, not a pass.
    assert!(matches!(
        parsed.validate(1_000_000, &mut |_: &mut [u8]| Err(())),
        Err(HeaderError::BadAppCrc)
    ));
}

/// The app is CRC'd in 256-byte chunks, so a length that is not a multiple of
/// the buffer exercises the final partial read.
#[test]
fn validate_handles_lengths_around_the_chunk_boundary() {
    for len in [1usize, 255, 256, 257, 512, 1000] {
        let app: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
        let parsed = HeaderInfo::from_bytes(&header_for(&app, AppType::Dashboard)).unwrap();
        parsed
            .validate(1_000_000, &mut reader(&app))
            .unwrap_or_else(|e| panic!("length {len} must validate, got {e:?}"));
    }
}

/// The flash tool computes this CRC and an already-flashed bootloader verifies
/// it. Swapping the algorithm (CRC-32/ISO-HDLC is the usual mix-up) would make
/// every new image fail validation on boards already in the field, so the
/// choice is pinned to its published check value.
#[test]
fn compute_crc_is_crc32_iscsi() {
    assert_eq!(compute_crc(b"123456789"), 0xE306_9283);
    assert_eq!(compute_crc(b""), 0);
}

/// `ValidationResult` is sent as `result as u8` by the bootloader and decoded
/// by a separate enum in the flash tool that hardcodes 0..=4. Two independent
/// declarations, one wire format.
#[test]
fn validation_result_codes_match_the_documented_numbering() {
    assert_eq!(ValidationResult::Valid as u8, 0);
    assert_eq!(ValidationResult::BadMagic as u8, 1);
    assert_eq!(ValidationResult::BadLength as u8, 2);
    assert_eq!(ValidationResult::BadCrc as u8, 3);
    assert_eq!(ValidationResult::WrongAppType as u8, 4);
}

/// The app type byte travels in the header, in the `.app_type` ELF section and
/// in the state response, and indexes the CAN address block.
#[test]
fn app_type_byte_values_match_the_documented_table() {
    assert_eq!(AppType::RudderController as u8, 0x01);
    assert_eq!(AppType::HeightSensorController as u8, 0x02);
    assert_eq!(AppType::Dashboard as u8, 0x03);
    for t in [
        AppType::RudderController,
        AppType::HeightSensorController,
        AppType::Dashboard,
    ] {
        assert_eq!(AppType::from_u8(t as u8), Some(t));
    }
    assert_eq!(AppType::from_u8(0x00), None);
    assert_eq!(AppType::from_u8(0x04), None);
}
