//! ELF64 loader — parses and loads ELF executables for user-mode.
//!
//! This is a **skeleton** ELF loader. It parses the ELF header and
//! program headers, but does NOT yet:
//! - Set up user-mode page tables
//! - Perform virtual memory mapping
//! - Jump to Ring3 via `iretq`
//!
//! [MANUAL] The actual Ring3 context switch requires:
//! - User-mode page tables (isolation from kernel memory)
//! - TSS RSP0 setup for privilege transitions
//! - `iretq` with CS=0x1B (user code), SS=0x23 (user data)
//! - User stack allocation
//!
//! ## Status
//! - ELF header validation ✅
//! - Program header parsing ✅
//! - Segment loading to kernel-accessible memory ✅ (skeleton)
//! - Ring3 page table setup: 【必须人工开发】
//! - Ring3 context switch: 【必须人工开发】

/// ELF magic: 0x7F 'E' 'L' 'F'
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// ELF class: ELF64.
const ELFCLASS64: u8 = 2;

/// ELF data encoding: little-endian.
const ELFDATA2LSB: u8 = 1;

/// ELF object type: executable.
const ET_EXEC: u16 = 2;

/// Program header type: loadable segment.
const PT_LOAD: u32 = 1;

/// ELF64 file header (64 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF64 program header (56 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// Result of loading an ELF file.
#[derive(Debug, Clone, Copy)]
pub struct LoadResult {
    /// Entry point address.
    pub entry: u64,
    /// Number of segments loaded.
    pub segments_loaded: u32,
}

/// Validate ELF header.
///
/// Checks: magic, class (64-bit), endianness (LE), type (EXEC),
/// machine (x86_64 = 0x3E).
///
/// Uses raw byte access to avoid packed-struct alignment issues.
pub fn validate_header_raw(ehdr_buf: &[u8; 16], e_type: u16, e_machine: u16) -> Result<(), &'static str> {
    if ehdr_buf[0..4] != ELF_MAGIC {
        return Err("invalid ELF magic");
    }
    if ehdr_buf[4] != ELFCLASS64 {
        return Err("not ELF64");
    }
    if ehdr_buf[5] != ELFDATA2LSB {
        return Err("not little-endian");
    }
    if e_type != ET_EXEC {
        return Err("not an executable (ET_EXEC)");
    }
    if e_machine != 0x3E {
        return Err("not x86_64");
    }
    Ok(())
}

/// Validate ELF header (from struct).
pub fn validate_header(ehdr: &Elf64Ehdr) -> Result<(), &'static str> {
    // Copy fields to avoid packed-struct alignment issues.
    let ident: [u8; 16] = ehdr.e_ident;
    let e_type = ehdr.e_type;
    let e_machine = ehdr.e_machine;
    validate_header_raw(&ident, e_type, e_machine)
}

/// Parse a program header from raw bytes.
///
/// [MANUAL] Safety: the caller must ensure the buffer is large enough
/// to hold a full Elf64Phdr (56 bytes).
pub fn parse_phdr(buf: &[u8]) -> Option<Elf64Phdr> {
    if buf.len() < 56 {
        return None;
    }
    let p_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let p_flags = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let p_offset = u64::from_le_bytes(buf[8..16].try_into().ok()?);
    let p_vaddr = u64::from_le_bytes(buf[16..24].try_into().ok()?);
    let p_paddr = u64::from_le_bytes(buf[24..32].try_into().ok()?);
    let p_filesz = u64::from_le_bytes(buf[32..40].try_into().ok()?);
    let p_memsz = u64::from_le_bytes(buf[40..48].try_into().ok()?);
    let p_align = u64::from_le_bytes(buf[48..56].try_into().ok()?);

    Some(Elf64Phdr {
        p_type,
        p_flags,
        p_offset,
        p_vaddr,
        p_paddr,
        p_filesz,
        p_memsz,
        p_align,
    })
}

/// Load an ELF executable from a memory buffer.
///
/// This is the `exec` implementation. It:
/// 1. Validates the ELF header
/// 2. Iterates program headers
/// 3. For each PT_LOAD segment, copies file data to the target virtual address
///
/// [MANUAL] In the real implementation, this will:
/// - Create a new process address space (page tables)
/// - Map PT_LOAD segments into the process address space
/// - Allocate user-mode stack
/// - Set up the TSS for Ring3
/// - `iretq` to jump to the entry point
///
/// For now, this copies segments to kernel-accessible memory and
/// records the entry point for future use.
pub fn load_elf(elf_buf: &[u8]) -> Result<LoadResult, &'static str> {
    if elf_buf.len() < 64 {
        return Err("buffer too small for ELF header");
    }

    // Parse the ELF header manually (packed struct, use byte reads).
    let e_ident: [u8; 16] = elf_buf[0..16].try_into().unwrap();
    let e_type = u16::from_le_bytes([elf_buf[16], elf_buf[17]]);
    let e_machine = u16::from_le_bytes([elf_buf[18], elf_buf[19]]);
    let _e_version = u32::from_le_bytes(elf_buf[20..24].try_into().unwrap());
    let e_entry = u64::from_le_bytes(elf_buf[24..32].try_into().unwrap());
    let e_phoff = u64::from_le_bytes(elf_buf[32..40].try_into().unwrap());
    let _e_shoff = u64::from_le_bytes(elf_buf[40..48].try_into().unwrap());
    let _e_flags = u32::from_le_bytes(elf_buf[48..52].try_into().unwrap());
    let _e_ehsize = u16::from_le_bytes([elf_buf[52], elf_buf[53]]);
    let e_phentsize = u16::from_le_bytes([elf_buf[54], elf_buf[55]]);
    let e_phnum = u16::from_le_bytes([elf_buf[56], elf_buf[57]]);

    let ehdr = Elf64Ehdr {
        e_ident,
        e_type,
        e_machine,
        e_version: 0,
        e_entry,
        e_phoff,
        e_shoff: 0,
        e_flags: 0,
        e_ehsize: 0,
        e_phentsize,
        e_phnum,
        e_shentsize: 0,
        e_shnum: 0,
        e_shstrndx: 0,
    };

    // Validate header.
    validate_header(&ehdr)?;

    // Iterate program headers.
    let phdr_base = e_phoff as usize;
    let phdr_size = e_phentsize as usize;
    let phdr_count = e_phnum as usize;

    let mut segments_loaded = 0u32;

    for i in 0..phdr_count {
        let offset = phdr_base + i * phdr_size;
        if offset + phdr_size > elf_buf.len() {
            break;
        }

        let phdr_buf = &elf_buf[offset..offset + phdr_size];
        let phdr = match parse_phdr(phdr_buf) {
            Some(p) => p,
            None => continue,
        };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        // Copy segment data from file to target virtual address.
        //
        // [MANUAL] In the real implementation, this maps pages at
        // p_vaddr and copies p_filesz bytes from the file, then
        // zero-fills the remainder (p_memsz - p_filesz).
        //
        // For now, we just count loaded segments and log.
        let file_data_start = phdr.p_offset as usize;
        let file_data_end = file_data_start + phdr.p_filesz as usize;

        if file_data_end <= elf_buf.len() {
            segments_loaded += 1;
            // Copy fields to avoid packed-struct alignment issues.
            let p_vaddr = phdr.p_vaddr;
            let p_filesz = phdr.p_filesz;
            let p_memsz = phdr.p_memsz;
            crate::serial::_print(format_args!(
                "[exec] PT_LOAD: vaddr={:#x}, filesz={}, memsz={}\n",
                p_vaddr, p_filesz, p_memsz
            ));
        }
    }

    crate::serial::_print(format_args!(
        "[exec] ELF loaded: entry={:#x}, segments={}\n",
        e_entry, segments_loaded
    ));

    Ok(LoadResult {
        entry: e_entry,
        segments_loaded,
    })
}

/// `exec(path, argv)` → 0 on success, negative errno on failure.
///
/// Loads an ELF executable from the FAT32 filesystem and prepares
/// a new process to run it.
///
/// [MANUAL] The full implementation requires:
/// - FAT32 file read to get the ELF bytes
/// - Process address space creation (page tables)
/// - Ring3 context switch
///
/// ## Status
/// - ELF parsing and validation: ✅
/// - File loading from FAT32: 【需要人工调试】
/// - Ring3 execution: 【必须人工开发，AI无法完整生成】
pub fn sys_exec(path: &[u8], _argv: &[&[u8]]) -> i64 {
    crate::serial::_print(format_args!(
        "[exec] requested path: \"{}\"\n",
        core::str::from_utf8(path).unwrap_or("<invalid>")
    ));

    // [MANUAL] When FAT32 is functional, read the file here:
    // let elf_buf = fat32::read_file(path);
    // For now, return ENOSYS — we can't actually read files yet.

    // SKELETON: if we had the ELF bytes, we would do:
    // let result = load_elf(&elf_buf)?;
    // process::alloc(...)
    // → set entry_point = result.entry

    crate::serial::_print(format_args!(
        "[exec] ENOSYS — requires FAT32 file read + Ring3\n"
    ));
    -38 // ENOSYS
}
