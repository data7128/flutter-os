//! User-mode process spawning — ELF loading into a user address space.
//!
//! This module replaces the skeleton ELF loader with a real implementation:
//! 1. Create a per-process address space (PML4 with kernel higher-half)
//! 2. Map ELF PT_LOAD segments into user space
//! 3. Allocate and map a user stack
//! 4. Set up the process kernel stack with a trampoline that `iretq`s
//!    into Ring3 at the ELF entry point
//!
//! The `spawn_user_process` function is the main entry point. It returns
//! the PID of the new process, which can then be scheduled.

use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

use crate::memory::page_table::AddressSpace;
use crate::process::{ProcessState, PROCESS_TABLE};
use crate::scheduler;

/// User-mode virtual address layout.
///
/// User space occupies the lower half (addresses < 0x0000_8000_0000_0000).
/// Kernel occupies the higher half. We place:
/// - ELF segments at their requested virtual addresses (typically low)
/// - User stack at a high user address, growing down
pub const USER_STACK_TOP: u64 = 0x0000_7FFF_0000_0000;
pub const USER_STACK_SIZE: u64 = 64 * 1024; // 64 KiB

/// Size of the red zone below the stack top (we leave one page unmapped
/// as a guard page to catch stack overflows).
pub const USER_STACK_GUARD: u64 = 4096;

/// Spawn a user-mode process from an ELF binary in memory.
///
/// # Arguments
/// - `elf_data`: raw ELF file bytes
/// - `name`: process name (for `ps` display)
///
/// # Returns
/// - `Ok(pid)` on success
/// - `Err(&str)` on failure
/// Load an ELF into a freshly created user address space: parse, create
/// the space, map every PT_LOAD segment and map the user stack.
///
/// Returns (address_space, entry_point, stack_top). Used by both `spawn`
/// (new process) and `exec` (replace current process image).
pub(crate) fn load_elf_into_new_space(
    elf_data: &[u8],
) -> Result<(AddressSpace, u64, u64), &'static str> {
    let (entry, segments) = parse_elf_segments(elf_data)?;

    crate::serial::_print(format_args!(
        "[exec] loading ELF: entry={:#x}, {} segments, data_len={}\n",
        entry,
        segments.len(),
        elf_data.len()
    ));
    for (i, sg) in segments.iter().enumerate() {
        crate::serial::_print(format_args!(
            "[exec]   seg{} vaddr={:#x} off={:#x} filesz={:#x} memsz={:#x} fl={:#x}\n",
            i, sg.vaddr, sg.offset, sg.filesz, sg.memsz, sg.flags
        ));
    }

    let addr_space = AddressSpace::new_user()?;
    for seg in &segments {
        map_elf_segment(&addr_space, elf_data, seg)?;
    }
    let stack_top = map_user_stack(&addr_space)?;
    Ok((addr_space, entry, stack_top))
}

pub fn spawn_user_process(elf_data: &[u8], name: &[u8]) -> Result<u32, &'static str> {
    // 1. Parse and validate the ELF header.
    let (entry, segments) = parse_elf_segments(elf_data)?;

    crate::serial::_print(format_args!(
        "[spawn] loading ELF: entry={:#x}, {} segments\n",
        entry,
        segments.len()
    ));

    // 2. Create a new user address space.
    let addr_space = AddressSpace::new_user()?;

    // 3. Map each PT_LOAD segment into the user address space.
    for seg in &segments {
        map_elf_segment(&addr_space, elf_data, seg)?;
    }

    // 4. Allocate and map the user stack.
    let stack_top = map_user_stack(&addr_space)?;

    // 5. Allocate a process slot.
    let pid = {
        let mut table = PROCESS_TABLE.lock();
        let pid = table.alloc(0, name); // parent = kernel (0)
        if pid == 0 {
            return Err("process table full");
        }
        if let Some(proc) = table.get_mut(pid) {
            proc.entry_point = entry;
            proc.user_rsp = stack_top;
            proc.cr3 = addr_space.pml4_frame.start_address().as_u64();
            proc.state = ProcessState::Ready;
        }
        pid
    };

    // 6. Set up the process kernel stack with the user-mode trampoline.
    let slot = PROCESS_TABLE
        .lock()
        .processes
        .iter()
        .position(|p| p.pid == pid)
        .ok_or("process slot not found")?;

    // The trampoline is the first function executed when the process is
    // scheduled. It runs in Ring0 and performs the iretq to Ring3.
    scheduler::init_process_stack(slot, user_trampoline as *const () as u64);

    // Store the address space pointer in the process for later switching.
    // For now, we leak it (it lives for the process lifetime).
    // In a full implementation, Process would own the AddressSpace.
    let addr_space_box = alloc::boxed::Box::new(addr_space);
    // The address space lives for the process lifetime; we leak it here.
    // TODO: add address_space field to Process struct and clean up on exit.
    core::mem::forget(addr_space_box);

    crate::serial::_print(format_args!(
        "[spawn] pid={} ready, entry={:#x}, stack={:#x}\n",
        pid, entry, stack_top
    ));

    Ok(pid)
}

/// Parsed ELF loadable segment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ElfSegment {
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32, // PT_LOAD p_flags: PF_X=1, PF_W=2, PF_R=4
}

/// Parse ELF header and return entry point + list of PT_LOAD segments.
pub(crate) fn parse_elf_segments(elf_data: &[u8]) -> Result<(u64, alloc::vec::Vec<ElfSegment>), &'static str> {
    if elf_data.len() < 64 {
        return Err("ELF too small");
    }

    // Magic check.
    if &elf_data[0..4] != &[0x7F, b'E', b'L', b'F'] {
        return Err("bad ELF magic");
    }
    if elf_data[4] != 2 {
        return Err("not ELF64");
    }
    if elf_data[5] != 1 {
        return Err("not little-endian");
    }

    let e_type = u16::from_le_bytes([elf_data[16], elf_data[17]]);
    if e_type != 2 {
        return Err("not ET_EXEC");
    }
    let e_machine = u16::from_le_bytes([elf_data[18], elf_data[19]]);
    if e_machine != 0x3E {
        return Err("not x86_64");
    }

    let e_entry = u64::from_le_bytes(elf_data[24..32].try_into().unwrap());
    let e_phoff = u64::from_le_bytes(elf_data[32..40].try_into().unwrap());
    let e_phentsize = u16::from_le_bytes([elf_data[54], elf_data[55]]);
    let e_phnum = u16::from_le_bytes([elf_data[56], elf_data[57]]);

    let mut segments = alloc::vec::Vec::new();

    for i in 0..e_phnum as usize {
        let off = e_phoff as usize + i * e_phentsize as usize;
        if off + 56 > elf_data.len() {
            break;
        }
        let p_type = u32::from_le_bytes(elf_data[off..off + 4].try_into().unwrap());
        if p_type != 1 {
            // PT_LOAD = 1
            continue;
        }
        let p_flags = u32::from_le_bytes(elf_data[off + 4..off + 8].try_into().unwrap());
        let p_offset = u64::from_le_bytes(elf_data[off + 8..off + 16].try_into().unwrap());
        let p_vaddr = u64::from_le_bytes(elf_data[off + 16..off + 24].try_into().unwrap());
        let p_filesz = u64::from_le_bytes(elf_data[off + 32..off + 40].try_into().unwrap());
        let p_memsz = u64::from_le_bytes(elf_data[off + 40..off + 48].try_into().unwrap());

        segments.push(ElfSegment {
            vaddr: p_vaddr,
            offset: p_offset,
            filesz: p_filesz,
            memsz: p_memsz,
            flags: p_flags,
        });
        crate::serial::_print(format_args!(
            "[parse] phdr[{}] type={} vaddr={:#x} off={:#x} filesz={:#x} memsz={:#x} fl={:#x}\n",
            i, p_type, p_vaddr, p_offset, p_filesz, p_memsz, p_flags
        ));
    }

    crate::serial::_print(format_args!(
        "[parse] e_phoff={} e_phentsize={} e_phnum={} data_len={}\n",
        e_phoff, e_phentsize, e_phnum, elf_data.len()
    ));
    Ok((e_entry, segments))
}

/// Map an ELF segment into the user address space.
///
/// Allocates physical frames for each page in the segment, maps them at
/// the segment's virtual address, copies the file data, and zero-fills
/// the BSS portion (memsz - filesz).
pub(crate) fn map_elf_segment(
    addr_space: &AddressSpace,
    elf_data: &[u8],
    seg: &ElfSegment,
) -> Result<(), &'static str> {
    if seg.memsz == 0 {
        return Ok(());
    }

    // Page-align the start and end.
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(seg.vaddr));
    let end_addr = VirtAddr::new(seg.vaddr + seg.memsz);
    let end_page = Page::<Size4KiB>::containing_address(end_addr);

    // Determine page flags from ELF segment flags.
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if seg.flags & 2 != 0 {
        // PF_W = writable
        flags |= PageTableFlags::WRITABLE;
    }
    if seg.flags & 1 == 0 {
        // PF_X not set → not executable (NX bit)
        // x86_64: NX is default if not set; we don't need to do anything.
        // Actually, PageTableFlags doesn't have EXECUTABLE; NX is the absence.
        // We leave it as-is (pages are executable by default unless NX set).
    }

    // Iterate over each page in the segment.
    for page in Page::range_inclusive(start_page, end_page) {
        // Reuse an already-mapped page (overlapping ELF segments) or
        // allocate a fresh frame. ELF loaders must handle segments that
        // share a page (e.g. the compiler emitting a text page that also
        // contains the start of .rodata).
        let tr = addr_space.translate(page.start_address().as_u64());
        let reused = tr.is_some();
        let frame = match tr {
            Some(pa) => x86_64::structures::paging::PhysFrame::<Size4KiB>::from_start_address(pa)
                .map_err(|_| "failed to map ELF page")?,
            None => addr_space
                .map_user_page_alloc(page, flags)
                .map_err(|_| "failed to map ELF page")?,
        };

        // Zero a freshly allocated frame. IMPORTANT: do NOT zero a reused
        // frame — an overlapping segment (e.g. .text page that also holds
        // the start of .rodata) already copied its data there; zeroing it
        // again would wipe the code and make the user trip on 0x00 bytes.
        let phys_offset = *crate::memory::page_table::PHYSICAL_OFFSET.lock();
        let frame_virt = VirtAddr::new(phys_offset + frame.start_address().as_u64());
        if !reused {
            unsafe {
                core::ptr::write_bytes(frame_virt.as_mut_ptr::<u8>(), 0, 4096);
            }
        }

        // Calculate how much file data goes into this page.
        let page_start_vaddr = page.start_address().as_u64();
        let page_end_vaddr = page_start_vaddr + 4096;

        // File data range for this page.
        let file_start_in_seg = if page_start_vaddr > seg.vaddr {
            page_start_vaddr - seg.vaddr
        } else {
            0
        };
        let file_end_in_seg = if page_end_vaddr > seg.vaddr + seg.filesz {
            seg.filesz
        } else {
            page_end_vaddr - seg.vaddr
        };

        if file_end_in_seg > file_start_in_seg && file_start_in_seg < seg.filesz {
            let src_offset = seg.offset + file_start_in_seg;
            let copy_len = (file_end_in_seg - file_start_in_seg) as usize;

            if (src_offset as usize + copy_len) <= elf_data.len() {
                // Destination offset within this page's frame = the
                // page-internal offset of the data start, i.e.
                // (data_start_vaddr & 0xfff). Two past bugs here:
                //   1. Using the in-segment offset (page_start - seg.vaddr)
                //      — for the 2nd+ page of a segment that is >= 0x1000,
                //      writing past the frame into the next physical frame
                //      and clobbering the page-table frame below.
                //   2. Using 0 unconditionally — for a non-page-aligned
                //      segment start (e.g. .rodata at 0x401fa0), the data
                //      was written at frame offset 0, overwriting the code
                //      at the start of that page.
                let data_start_vaddr = if page_start_vaddr < seg.vaddr {
                    seg.vaddr
                } else {
                    page_start_vaddr
                };
                let dst_offset = (data_start_vaddr & 0xfff) as usize;
                let dst_ptr = unsafe { frame_virt.as_mut_ptr::<u8>().add(dst_offset) };
                let src_ptr = unsafe { elf_data.as_ptr().add(src_offset as usize) };
                unsafe {
                    core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, copy_len);
                }
            }
        }
        // Remainder of the page (BSS) is already zeroed.
    }

    Ok(())
}

/// Allocate and map the user stack.
///
/// Maps `USER_STACK_SIZE` bytes at `USER_STACK_TOP - size`, with a guard
/// page below. Returns the stack top (initial RSP).
pub(crate) fn map_user_stack(addr_space: &AddressSpace) -> Result<u64, &'static str> {
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    let stack_bottom = USER_STACK_TOP - USER_STACK_SIZE;
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(stack_bottom));
    // Map the top page too: user entry code (e.g. sysutils `_start`)
    // reads argc/argv from [rsp] with rsp == USER_STACK_TOP, so that
    // address must be readable/writable. USER_STACK_TOP is page-aligned,
    // so this adds exactly one page.
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(USER_STACK_TOP));

    for page in Page::range_inclusive(start_page, end_page) {
        addr_space
            .map_user_page_alloc(page, flags)
            .map_err(|_| "failed to map user stack page")?;
    }

    // The guard page (one page below stack_bottom) is left unmapped.
    // This causes a page fault on stack overflow.

    Ok(USER_STACK_TOP)
}

/// Kernel-side trampoline for entering user mode.
///
/// This is the first function executed when a new user process is
/// scheduled. It runs in Ring0, looks up the current process's entry
/// point and user stack, then calls `enter_usermode` to `iretq` into
/// Ring3.
///
/// # Safety
/// This function is called as the "return address" from the context
/// switch assembly. It must never return — `enter_usermode` is noreturn.
pub(crate) extern "C" fn user_trampoline() -> ! {
    let (entry, user_rsp, cr3) = {
        let table = PROCESS_TABLE.lock();
        let pid = table.current_pid;
        let proc = table
            .get(pid)
            .expect("user_trampoline: current process not found");
        (proc.entry_point, proc.user_rsp, proc.cr3)
    };

    crate::serial::_print(format_args!(
        "[trampoline] entering Ring3: entry={:#x}, rsp={:#x}, cr3={:#x}\n",
        entry, user_rsp, cr3
    ));

    // Activate the process's own address space (user PML4). Without this
    // the user pages (ELF segments, stack) are not mapped and the first
    // instruction fetch faults.
    if cr3 != 0 {
        use x86_64::structures::paging::{PhysFrame, Size4KiB};
        let frame = PhysFrame::<Size4KiB>::from_start_address(x86_64::PhysAddr::new(cr3))
            .expect("trampoline: invalid cr3");
        let addr_space = crate::memory::page_table::AddressSpace { pml4_frame: frame };
        unsafe {
            addr_space.switch_to();
        }
    }

    unsafe {
        crate::syscall_trampoline::enter_usermode(entry, user_rsp);
    }
}

/// Spawn a user process from an ELF file at a VFS path.
///
/// Reads the entire ELF from the filesystem, then delegates to
/// `spawn_user_process`. This is the implementation behind the `execve`
/// syscall and the kernel's own test launches.
pub fn spawn_user_process_from_path(path: &str) -> Result<u64, &'static str> {
    let elf_data = crate::fs::VFS.lock().read_all(path)?;
    if elf_data.is_empty() {
        return Err("ELF file is empty");
    }
    // Use the file name (last component) as the process name.
    let name = path.rsplit('/').next().unwrap_or(path);
    let pid = spawn_user_process(&elf_data, name.as_bytes())?;
    Ok(pid as u64)
}
