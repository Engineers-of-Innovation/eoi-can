use eoi_boot_api::header::{self, AppType};
use std::path::Path;

/// The app partition starts here in flash. Keep in sync with
/// `linker/app.x` (FLASH origin) and `boot/src/flash.rs` (app_offset).
const EXPECTED_APP_BASE_ADDRESS: usize = 0x0801_4800;

/// Firmware image ready to be flashed (header + raw app binary).
pub struct FirmwareImage {
    /// The complete blob: 2048-byte header followed by the raw app binary.
    pub data: Vec<u8>,
    /// Size of just the app binary (without header).
    pub app_size: usize,
    /// Application type declared by the binary (via the `.app_type` ELF section).
    pub app_type: AppType,
}

impl FirmwareImage {
    /// Parse an ELF file and produce a firmware image (header + app binary).
    pub fn from_elf_file(path: impl AsRef<Path>) -> Result<Self, ElfError> {
        let data = std::fs::read(path.as_ref()).map_err(ElfError::Io)?;
        Self::from_elf_bytes(&data)
    }

    /// Parse ELF bytes and produce a firmware image.
    pub fn from_elf_bytes(input: &[u8]) -> Result<Self, ElfError> {
        let elf = elf::ElfBytes::<elf::endian::LittleEndian>::minimal_parse(input)
            .map_err(ElfError::Parse)?;

        // Validate it's an ARM executable
        if elf.ehdr.e_type != elf::abi::ET_EXEC {
            return Err(ElfError::NotExecutable(elf.ehdr.e_type));
        }
        if elf.ehdr.e_machine != elf::abi::EM_ARM {
            return Err(ElfError::NotArm(elf.ehdr.e_machine));
        }

        // Read application type from the dedicated ELF section.
        let app_type = extract_app_type(&elf, input)?;

        // Extract loadable segments into a raw memory image
        let raw_image = extract_loadable_segments(&elf, input)?;
        log::info!("Raw firmware image: {} bytes", raw_image.len());

        // Compute CRC32 and build header
        let app_crc = header::compute_crc(&raw_image);
        let hdr = header::build_header(raw_image.len() as u32, app_crc, app_type);

        // Combine header + raw image
        let mut blob = Vec::with_capacity(header::HEADER_PARTITION_SIZE + raw_image.len());
        blob.extend_from_slice(&hdr);
        blob.extend_from_slice(&raw_image);

        Ok(Self {
            app_size: raw_image.len(),
            data: blob,
            app_type,
        })
    }
}

/// Locate the `.app_type` section in the ELF and decode the single byte it carries.
fn extract_app_type(
    elf: &elf::ElfBytes<elf::endian::LittleEndian>,
    input: &[u8],
) -> Result<AppType, ElfError> {
    let section = elf
        .section_header_by_name(".app_type")
        .map_err(ElfError::Parse)?
        .ok_or(ElfError::MissingAppType)?;

    let start = section.sh_offset as usize;
    let end = start
        .checked_add(section.sh_size as usize)
        .ok_or(ElfError::MissingAppType)?;
    let bytes = input.get(start..end).ok_or(ElfError::MissingAppType)?;

    if bytes.len() != 1 {
        return Err(ElfError::BadAppType(bytes.to_vec()));
    }
    AppType::from_u8(bytes[0]).ok_or(ElfError::BadAppType(bytes.to_vec()))
}

/// Extract all PT_LOAD segments from an ELF into a contiguous memory image.
fn extract_loadable_segments(
    elf: &elf::ElfBytes<elf::endian::LittleEndian>,
    input: &[u8],
) -> Result<Vec<u8>, ElfError> {
    let segments = elf.segments().ok_or(ElfError::NoLoadableSegments)?;

    // Find the address range
    let mut lowest_addr = usize::MAX;
    let mut highest_addr = 0;
    for segment in segments.iter() {
        if segment.p_type != elf::abi::PT_LOAD || segment.p_filesz == 0 {
            continue;
        }
        let mem_start = segment.p_paddr as usize;
        let mem_end = mem_start + segment.p_filesz as usize;
        lowest_addr = lowest_addr.min(mem_start);
        highest_addr = highest_addr.max(mem_end);
    }

    if lowest_addr >= highest_addr {
        return Err(ElfError::NoLoadableSegments);
    }

    log::info!(
        "ELF load address range: 0x{:08X} - 0x{:08X} ({} bytes)",
        lowest_addr,
        highest_addr,
        highest_addr - lowest_addr
    );

    if lowest_addr != EXPECTED_APP_BASE_ADDRESS {
        return Err(ElfError::WrongLoadAddress {
            found: lowest_addr,
            expected: EXPECTED_APP_BASE_ADDRESS,
        });
    }

    // Build the raw image
    let mut output = vec![0x00_u8; highest_addr - lowest_addr];
    for segment in segments.iter() {
        if segment.p_type != elf::abi::PT_LOAD || segment.p_filesz == 0 {
            continue;
        }
        let data_start = segment.p_offset as usize;
        let data_end = data_start + segment.p_filesz as usize;
        let data = input
            .get(data_start..data_end)
            .ok_or(ElfError::SegmentOutOfBounds {
                start: data_start,
                end: data_end,
                file_size: input.len(),
            })?;

        let output_start = segment.p_paddr as usize - lowest_addr;
        output[output_start..][..segment.p_filesz as usize].copy_from_slice(data);
    }

    Ok(output)
}

#[derive(Debug, thiserror::Error)]
pub enum ElfError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ELF parse error: {0}")]
    Parse(elf::ParseError),
    #[error("ELF is not an executable (type {0})")]
    NotExecutable(u16),
    #[error("ELF is not for ARM (machine {0})")]
    NotArm(u16),
    #[error("No loadable segments found")]
    NoLoadableSegments,
    #[error("Segment out of bounds: {start}..{end} exceeds file size {file_size}")]
    SegmentOutOfBounds {
        start: usize,
        end: usize,
        file_size: usize,
    },
    #[error(
        "ELF is missing the `.app_type` section — binary must call `declare_app_type!(...)` (in eoi-firmware::app_type)"
    )]
    MissingAppType,
    #[error("`.app_type` section has unexpected contents: {0:02X?}")]
    BadAppType(Vec<u8>),
    #[error(
        "ELF is linked at 0x{found:08X} but the app partition starts at 0x{expected:08X} — rebuild the app with `--features bootloader`"
    )]
    WrongLoadAddress { found: usize, expected: usize },
}
