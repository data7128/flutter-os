//! Page table management — x86_64 4-level paging.
//!
//! Wraps the `x86_64` crate's `OffsetPageTable` to provide:
//! - Kernel page table initialisation from the bootloader's physical offset
//! - `map` / `unmap` helpers for kernel and user address spaces
//! - Per-process address space creation (new PML4 table)
//! - Active page table switching (`cr3` write)

use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame,
    Size4KiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

use spin::Mutex;

/// The kernel's active page table, initialised once at boot.
///
/// `OffsetPageTable` knows the physical→virtual offset so it can
/// dereference page-table physical addresses through the higher-half
/// mapping that the bootloader set up.
pub static KERNEL_PAGE_TABLE: Mutex<Option<OffsetPageTable<'static>>> = Mutex::new(None);

/// Physical memory offset (virtual = physical + offset), supplied by
/// the bootloader. Stored here so address-space creation can build new
/// page tables that also map the kernel higher-half.
pub static PHYSICAL_OFFSET: Mutex<u64> = Mutex::new(0);

/// Initialise the kernel page table.
///
/// # Safety
/// Caller must ensure `physical_memory_offset` is the correct offset
/// supplied by the bootloader, and that the active PML4 is the one
/// the bootloader built (i.e. we haven't switched CR3 yet).
pub unsafe fn init(physical_memory_offset: VirtAddr) {
    *PHYSICAL_OFFSET.lock() = physical_memory_offset.as_u64();

    let level_4_table = active_level_4_table(physical_memory_offset);
    let pt = OffsetPageTable::new(level_4_table, physical_memory_offset);
    *KERNEL_PAGE_TABLE.lock() = Some(pt);

    crate::serial::_print(format_args!("[paging] kernel page table initialised\n"));
}

/// Get a mutable reference to the active PML4 table.
///
/// # Safety
/// `physical_memory_offset` must be correct.
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();
    &mut *page_table_ptr
}

/// Map a virtual page to a physical frame in the kernel address space.
pub fn map_to(
    page: Page<Size4KiB>,
    frame: PhysFrame<Size4KiB>,
    flags: PageTableFlags,
) -> Result<(), &'static str> {
    let mut pt_guard = KERNEL_PAGE_TABLE.lock();
    let pt = pt_guard.as_mut().ok_or("page table not initialised")?;
    let mut alloc = crate::memory::frame_allocator::FRAME_ALLOCATOR.lock();
    unsafe {
        pt.map_to(page, frame, flags, &mut *alloc)
            .map_err(|_| "map_to failed")?
            .flush();
    }
    Ok(())
}

/// Allocate a physical frame and map it at `page` with `flags`.
pub fn map_page_alloc(
    page: Page<Size4KiB>,
    flags: PageTableFlags,
) -> Result<PhysFrame<Size4KiB>, &'static str> {
    let frame = crate::memory::frame_allocator::alloc_frame().ok_or("out of physical frames")?;
    map_to(page, frame, flags)?;
    Ok(frame)
}

/// Unmap a virtual page (does not free the physical frame).
pub fn unmap(page: Page<Size4KiB>) -> Result<PhysFrame<Size4KiB>, &'static str> {
    let mut pt_guard = KERNEL_PAGE_TABLE.lock();
    let pt = pt_guard.as_mut().ok_or("page table not initialised")?;
    let (frame, flush) = pt.unmap(page).map_err(|_| "unmap failed")?;
    flush.flush();
    Ok(frame)
}

/// Translate a virtual address to a physical address in the active table.
pub fn translate_addr(addr: VirtAddr) -> Option<PhysAddr> {
    let pt_guard = KERNEL_PAGE_TABLE.lock();
    let pt = pt_guard.as_ref()?;
    pt.translate_addr(addr)
}

// ── Per-process address spaces ─────────────────────────────────────────

/// A user-mode address space: owns a PML4 table and can be activated.
pub struct AddressSpace {
    /// Physical frame containing the PML4 table.
    pub pml4_frame: PhysFrame<Size4KiB>,
}

impl AddressSpace {
    /// Create a new user address space.
    ///
    /// Allocates a fresh PML4 frame, zeroes it, then copies the kernel
    /// higher-half entries (entries 256..512) from the kernel PML4 so
    /// that kernel memory is accessible during syscalls / interrupts.
    pub fn new_user() -> Result<Self, &'static str> {
        let pml4_frame = crate::memory::frame_allocator::alloc_frame().ok_or("out of frames for PML4")?;

        let phys_offset = *PHYSICAL_OFFSET.lock();
        let pml4_virt = VirtAddr::new(phys_offset + pml4_frame.start_address().as_u64());

        // Zero the new PML4.
        unsafe {
            core::ptr::write_bytes(pml4_virt.as_mut_ptr::<u8>(), 0, 4096);
        }

        let new_pml4 = unsafe { &mut *(pml4_virt.as_mut_ptr::<PageTable>()) };

        // Copy the kernel's PML4 entries into the new user address space.
        //
        // The bootloader maps the kernel at 0x8000_0000_0000 — that is
        // 512 GiB, PML4 index 1 — and the physical-memory window at
        // 0x200_0000_0000 (2 TiB, PML4 index 4). We copy indices 1..512
        // so syscalls/interrupts can reach kernel code, data and the
        // per-process kernel stacks (all in the higher region).
        //
        // Index 0 (the user region, 0 .. 512 GiB) is intentionally NOT
        // copied: copying it would share the kernel's intermediate page
        // tables (bootloader stack, identity map), and mapping a user
        // page would then mutate the kernel's tables — the second
        // process to load at the same virtual address (e.g. 0x400000)
        // would collide with the first process's leftover mapping.
        let kernel_pml4_phys = x86_64::registers::control::Cr3::read().0.start_address();
        let kernel_pml4_virt = VirtAddr::new(phys_offset + kernel_pml4_phys.as_u64());
        let kernel_pml4 = unsafe { &*(kernel_pml4_virt.as_ptr::<PageTable>()) };

        for i in 1..512 {
            let e = kernel_pml4[i].clone();
            if !e.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if e.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
                // User region (e.g. the user stack at PML4 index 255).
                // Never inherit another process's user mapping here: new
                // user mappings are created by map_user_page, and fork
                // deep-copies them via clone_user_space.
                continue;
            }
            new_pml4[i] = e;
        }

        Ok(Self { pml4_frame })
    }

    /// Wrap an existing PML4 frame (e.g. a process's cr3) as an
    /// address space handle.
    pub fn from_cr3(cr3: u64) -> Option<Self> {
        if cr3 == 0 {
            return None;
        }
        let frame = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(cr3)).ok()?;
        Some(Self { pml4_frame: frame })
    }

    /// Deep-copy the user portion (PML4 entries 0..256) of this address
    /// space into a fresh one. Every present user page gets its own
    /// physical frame with the same contents and flags; kernel entries
    /// are re-copied by `new_user`.
    ///
    /// Used by `fork` so the child sees an identical user memory image.
    /// Copy-on-write is a future optimisation.
    pub fn clone_user_space(&self) -> Result<Self, &'static str> {
        let new = Self::new_user()?;
        let phys_offset = *PHYSICAL_OFFSET.lock();

        // Deep-copy the user portion of this address space into a fresh
        // one. User memory lives entirely in PML4 index 0 (0 .. 512 GiB;
        // ELF at 0x400000, stack at 0x7fff_0000_0000). Index 1..511 is
        // the kernel region (code at 0x8000_0000_0000, phys window at
        // 0x200_0000_0000) and is copied by `new_user` — deep-copying it
        // here would exhaust the frame allocator.
        let src_pml4_virt = VirtAddr::new(phys_offset + self.pml4_frame.start_address().as_u64());
        let src_pml4 = unsafe { &*(src_pml4_virt.as_ptr::<PageTable>()) };

        for pml4_idx in 0..512 {
            let l4 = src_pml4[pml4_idx].clone();
            if !l4.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if !l4.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
                // Kernel region (higher-half, phys window): shared, not
                // copied per-process.
                continue;
            }
            if l4.flags().contains(PageTableFlags::HUGE_PAGE) {
                return Err("1GiB huge page in user space");
            }
            let pdpt_frame = l4.frame().map_err(|_| "l4 entry without frame")?;
            let pdpt_virt = VirtAddr::new(phys_offset + pdpt_frame.start_address().as_u64());
            let pdpt = unsafe { &*(pdpt_virt.as_ptr::<PageTable>()) };

            for pdpt_idx in 0..512 {
                let l3 = pdpt[pdpt_idx].clone();
                if !l3.flags().contains(PageTableFlags::PRESENT) {
                    continue;
                }
                if l3.flags().contains(PageTableFlags::HUGE_PAGE) {
                    return Err("1GiB huge page in user space");
                }
                let pd_frame = l3.frame().map_err(|_| "l3 entry without frame")?;
                let pd_virt = VirtAddr::new(phys_offset + pd_frame.start_address().as_u64());
                let pd = unsafe { &*(pd_virt.as_ptr::<PageTable>()) };

                for pd_idx in 0..512 {
                    let l2 = pd[pd_idx].clone();
                    if !l2.flags().contains(PageTableFlags::PRESENT) {
                        continue;
                    }
                    if l2.flags().contains(PageTableFlags::HUGE_PAGE) {
                        return Err("2MiB huge page in user space");
                    }
                    let pt_frame = l2.frame().map_err(|_| "l2 entry without frame")?;
                    let pt_virt = VirtAddr::new(phys_offset + pt_frame.start_address().as_u64());
                    let pt = unsafe { &*(pt_virt.as_ptr::<PageTable>()) };

                    for pt_idx in 0..512 {
                        let l1 = pt[pt_idx].clone();
                        if !l1.flags().contains(PageTableFlags::PRESENT)
                            || !l1.flags().contains(PageTableFlags::USER_ACCESSIBLE)
                        {
                            continue;
                        }
                        let src_frame = l1.frame().map_err(|_| "l1 entry without frame")?;

                        let vaddr = (pml4_idx as u64) << 39
                            | (pdpt_idx as u64) << 30
                            | (pd_idx as u64) << 21
                            | (pt_idx as u64) << 12;
                        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(vaddr));

                        // Allocate a fresh frame in the new space and
                        // copy the page contents.
                        let dst_frame = new.map_user_page_alloc(page, l1.flags())?;
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                (phys_offset + src_frame.start_address().as_u64()) as *const u8,
                                (phys_offset + dst_frame.start_address().as_u64()) as *mut u8,
                                4096,
                            );
                        }
                    }
                }
            }
        }

        crate::serial::_print(format_args!(
            "[addrspace] cloned user space {:#x} → {:#x}\n",
            self.pml4_frame.start_address().as_u64(),
            new.pml4_frame.start_address().as_u64()
        ));

        Ok(new)
    }

    /// Switch to this address space (write CR3).
    ///
    /// # Safety
    /// Caller must ensure the PML4 is valid and the kernel higher-half
    /// entries are present, otherwise the next instruction will fault.
    pub unsafe fn switch_to(&self) {
        use x86_64::registers::control::Cr3;
        let (_, flags) = Cr3::read();
        Cr3::write(self.pml4_frame, flags);
    }

    /// Map a page in this address space.
    ///
    /// Temporarily maps the user PML4 (and intermediate tables) through
    /// the physical-offset window to manipulate them.
    pub fn map_user_page(
        &self,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
    ) -> Result<(), &'static str> {
        let phys_offset = *PHYSICAL_OFFSET.lock();
        let mut alloc = crate::memory::frame_allocator::FRAME_ALLOCATOR.lock();

        // We build a temporary OffsetPageTable pointing at the user PML4.
        // The user PML4 is accessible at phys_offset + pml4_phys.
        let pml4_virt = VirtAddr::new(phys_offset + self.pml4_frame.start_address().as_u64());
        let user_pml4 = unsafe { &mut *(pml4_virt.as_mut_ptr::<PageTable>()) };

        // Safety: we hold the frame allocator lock and the PML4 is valid.
        let mut mapper = unsafe { OffsetPageTable::new(user_pml4, VirtAddr::new(phys_offset)) };

        unsafe {
            match mapper.map_to(page, frame, flags, &mut *alloc) {
                Ok(flush) => {
                    flush.flush();
                }
                Err(e) => {
                    crate::serial::_print(format_args!(
                        "[map] map_to({:#x}->{:#x}) failed: {:?}\n",
                        page.start_address().as_u64(),
                        frame.start_address().as_u64(),
                        e
                    ));
                    return Err("user map_to failed");
                }
            }
        }
        Ok(())
    }

    /// Allocate a frame and map it at `page` in this address space.
    pub fn map_user_page_alloc(
        &self,
        page: Page<Size4KiB>,
        flags: PageTableFlags,
    ) -> Result<PhysFrame<Size4KiB>, &'static str> {
        let frame = match crate::memory::frame_allocator::alloc_frame() {
            Some(f) => f,
            None => {
                crate::serial::_print(format_args!(
                    "[addrspace] out of physical frames (page {:#x})\n",
                    page.start_address().as_u64()
                ));
                return Err("out of physical frames");
            }
        };
        if let Err(e) = self.map_user_page(page, frame, flags) {
            crate::serial::_print(format_args!(
                "[addrspace] map_user_page failed for {:#x}: {}\n",
                page.start_address().as_u64(),
                e
            ));
            return Err(e);
        }
        Ok(frame)
    }
}

impl AddressSpace {
    /// Translate a user virtual address through this address space's page
    /// tables, returning the physical address. None if unmapped.
    pub fn translate(&self, vaddr: u64) -> Option<PhysAddr> {
        let phys_offset = *PHYSICAL_OFFSET.lock();
        let l4i = ((vaddr >> 39) & 0x1ff) as usize;
        let l3i = ((vaddr >> 30) & 0x1ff) as usize;
        let l2i = ((vaddr >> 21) & 0x1ff) as usize;
        let l1i = ((vaddr >> 12) & 0x1ff) as usize;

        let pml4 = unsafe { &*(VirtAddr::new(phys_offset + self.pml4_frame.start_address().as_u64()).as_ptr::<PageTable>()) };
        let e4 = pml4[l4i].clone();
        if !e4.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        let pdpt = unsafe { &*(VirtAddr::new(phys_offset + e4.addr().as_u64()).as_ptr::<PageTable>()) };
        let e3 = pdpt[l3i].clone();
        if !e3.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        if e3.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Some(PhysAddr::new(e3.addr().as_u64() + (vaddr & 0x3fff_ffff)));
        }
        let pd = unsafe { &*(VirtAddr::new(phys_offset + e3.addr().as_u64()).as_ptr::<PageTable>()) };
        let e2 = pd[l2i].clone();
        if !e2.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        if e2.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Some(PhysAddr::new(e2.addr().as_u64() + (vaddr & 0x1f_ffff)));
        }
        let pt = unsafe { &*(VirtAddr::new(phys_offset + e2.addr().as_u64()).as_ptr::<PageTable>()) };
        let e1 = pt[l1i].clone();
        if !e1.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        Some(PhysAddr::new(e1.addr().as_u64() + (vaddr & 0xfff)))
    }

    /// Free every physical frame owned by this address space's USER region
    /// (PML4 index 0: user pages + their page tables + the PML4 itself).
    /// Kernel entries (indices 1..511) are shared and never freed here.
    ///
    /// Used when a process is reaped (waitpid) or replaced (exec).
    pub fn dealloc_user(&self) {
        let phys_offset = *PHYSICAL_OFFSET.lock();
        let dealloc = |pa: u64| {
            if let Ok(frame) = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(pa)) {
                crate::memory::frame_allocator::dealloc_frame(frame);
            }
        };

        let pml4_virt = VirtAddr::new(phys_offset + self.pml4_frame.start_address().as_u64());
        let pml4 = unsafe { &mut *(pml4_virt.as_mut_ptr::<PageTable>()) };

        // User region = PML4 index 0.
        let e4 = pml4[0].clone();
        if !e4.flags().contains(PageTableFlags::PRESENT) {
            dealloc(self.pml4_frame.start_address().as_u64());
            return;
        }
        let pdpt_virt = VirtAddr::new(phys_offset + e4.addr().as_u64());
        let pdpt = unsafe { &mut *(pdpt_virt.as_mut_ptr::<PageTable>()) };
        for l3i in 0..512 {
            let e3 = pdpt[l3i].clone();
            if !e3.flags().contains(PageTableFlags::PRESENT) {
                continue;
            }
            if e3.flags().contains(PageTableFlags::HUGE_PAGE) {
                // 1 GiB huge page — we never allocate these, skip.
                continue;
            }
            let pd_virt = VirtAddr::new(phys_offset + e3.addr().as_u64());
            let pd = unsafe { &mut *(pd_virt.as_mut_ptr::<PageTable>()) };
            for l2i in 0..512 {
                let e2 = pd[l2i].clone();
                if !e2.flags().contains(PageTableFlags::PRESENT) {
                    continue;
                }
                if e2.flags().contains(PageTableFlags::HUGE_PAGE) {
                    // 2 MiB huge page — we never allocate these, skip.
                    continue;
                }
                let pt_virt = VirtAddr::new(phys_offset + e2.addr().as_u64());
                let pt = unsafe { &mut *(pt_virt.as_mut_ptr::<PageTable>()) };
                for l1i in 0..512 {
                    let e1 = pt[l1i].clone();
                    if e1.flags().contains(PageTableFlags::PRESENT) {
                        dealloc(e1.addr().as_u64());
                    }
                }
                dealloc(e2.addr().as_u64()); // PT frame
            }
            dealloc(e3.addr().as_u64()); // PD frame
        }
        dealloc(e4.addr().as_u64()); // PDPT frame
        dealloc(self.pml4_frame.start_address().as_u64()); // PML4 frame
    }
}
