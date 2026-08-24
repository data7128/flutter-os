//! .aero application package format — prototype native app format.
//!
//! ## Package Structure
//!
//! A `.aero` file is a simple binary container:
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ Magic: "AERO" (4 bytes)                         │
//! ├─────────────────────────────────────────────────┤
//! │ Version: u16 (currently 1)                      │
//! ├─────────────────────────────────────────────────┤
//! │ App name length: u8                             │
//! ├─────────────────────────────────────────────────┤
//! │ App name: [u8; N] (N = name_length, max 31)    │
//! ├─────────────────────────────────────────────────┤
//! │ Icon color: u32 (0xRRGGBB)                     │
//! ├─────────────────────────────────────────────────┤
//! │ ELF binary length: u32                         │
//! ├─────────────────────────────────────────────────┤
//! │ ELF binary: [u8; M] (M = elf_length)           │
//! ├─────────────────────────────────────────────────┤
//! │ Resource count: u16                            │
//! ├─────────────────────────────────────────────────┤
//! │ Resource entries:                              │
//! │   [name_len: u8][name: u8*][data_len: u32]     │
//! │   [data: u8*N]                                 │
//! │   ... (repeated for each resource)             │
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! ## Limitations
//! - **No digital signatures**: packages are not signed or verified.
//!   Any process can create and load .aero packages.
//! - **No checksum**: no integrity verification (CRC/hash).
//! - **Prototype only**: this format may change between versions.
//!
//! [MANUAL] The full implementation requires:
//! - FAT32 file read to load .aero from disk
//! - ELF loader to parse the embedded ELF binary
//! - Ring3 exec to start the app process

/// Magic bytes for .aero format.
pub const AERO_MAGIC: [u8; 4] = *b"AERO";

/// Current format version.
pub const AERO_VERSION: u16 = 1;

/// Maximum app name length.
pub const MAX_NAME_LEN: usize = 31;

/// Aero package header (parsed from the binary).
#[derive(Debug, Clone, Copy)]
pub struct AeroHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub name_len: u8,
    pub icon_color: u32,
    pub elf_len: u32,
    pub resource_count: u16,
}

/// Parse the .aero header from a byte buffer.
///
/// Returns the parsed header and the offset where the app name starts.
pub fn parse_header(buf: &[u8]) -> Option<(AeroHeader, usize)> {
    if buf.len() < 4 + 2 + 1 {
        return None; // Too small for magic + version + name_len.
    }

    if &buf[0..4] != AERO_MAGIC {
        return None; // Bad magic.
    }

    let version = u16::from_le_bytes([buf[4], buf[5]]);
    let name_len = buf[6];

    // Header layout: magic(4) + version(2) + name_len(1) + name(N) + icon(4) + elf_len(4)
    let name_start = 7;
    let name_end = name_start + name_len as usize;
    if buf.len() < name_end + 4 + 4 + 2 {
        return None;
    }

    let icon_color = u32::from_le_bytes(
        buf[name_end..name_end + 4].try_into().ok()?
    );
    let elf_len = u32::from_le_bytes(
        buf[name_end + 4..name_end + 8].try_into().ok()?
    );
    let resource_count = u16::from_le_bytes([
        buf[name_end + 8],
        buf[name_end + 9],
    ]);

    let header = AeroHeader {
        magic: AERO_MAGIC,
        version,
        name_len,
        icon_color,
        elf_len,
        resource_count,
    };

    Some((header, name_start))
}

/// Extract the ELF binary offset from a parsed .aero package.
///
/// Returns (elf_offset, elf_len).
pub fn elf_offset(header: &AeroHeader) -> (usize, usize) {
    // magic(4) + version(2) + name_len(1) + name(N) + icon(4) + elf_len(4) + res_count(2)
    let offset = 7 + header.name_len as usize + 4 + 4 + 2;
    (offset, header.elf_len as usize)
}

/// Extract the app name from a parsed .aero package.
pub fn extract_name<'a>(buf: &'a [u8], header: &AeroHeader) -> &'a [u8] {
    let start = 7;
    let end = start + header.name_len as usize;
    if end > buf.len() {
        return b"";
    }
    &buf[start..end]
}

/// Load a .aero package from a byte buffer and prepare to exec it.
///
/// [MANUAL] The full implementation requires:
/// 1. FAT32 read to get the .aero bytes from disk
/// 2. Header parsing (done in skeleton)
/// 3. ELF extraction and loading via exec::load_elf
/// 4. Ring3 process creation and context switch
pub fn load_aero(buf: &[u8]) -> core::result::Result<&[u8], &'static str> {
    let (header, _) = parse_header(buf).ok_or("invalid .aero header")?;

    if header.version != AERO_VERSION {
        return Err("unsupported .aero version");
    }

    let (elf_off, elf_len) = elf_offset(&header);

    if elf_off + elf_len > buf.len() {
        return Err("ELF section truncated");
    }

    crate::serial::_print(format_args!(
        "[aero] loaded: name_len={}, elf_len={}, resources={}\n",
        header.name_len, header.elf_len, header.resource_count
    ));

    Ok(&buf[elf_off..elf_off + elf_len])
}
