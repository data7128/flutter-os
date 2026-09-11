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
        // The bootloader maps the kernel at 0x8000_0000_0000, which lives
        // in PML4 index 4 — NOT the conventional higher-half (index
        // 256..512) — so we copy the entire table. User pages are added
        // afterwards by map_user_page; supervisor PTE flags keep kernel
        // memory inaccessible from Ring3.
        let kernel_pml4_phys = x86_64::registers::control::Cr3::read().0.start_address();
        let kernel_pml4_virt = VirtAddr::new(phys_offset + kernel_pml4_phys.as_u64());
        let kernel_pml4 = unsafe { &*(kernel_pml4_virt.as_ptr::<PageTable>()) };

        for i in 0..512 {
            new_pml4[i] = kernel_pml4[i].clone();
        }

        crate::serial::_print(format_args!(
            "[addrspace] new user PML4 at phys={:#x}\n",
            pml4_frame.start_address().as_u64()
        ));

        Ok(Self { pml4_frame })
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
            mapper
                .map_to(page, frame, flags, &mut *alloc)
                .map_err(|_| "user map_to failed")?
                .flush();
        }
        Ok(())
    }

    /// Allocate a frame and map it at `page` in this address space.
    pub fn map_user_page_alloc(
        &self,
        page: Page<Size4KiB>,
        flags: PageTableFlags,
    ) -> Result<PhysFrame<Size4KiB>, &'static str> {
        let frame = crate::memory::frame_allocator::alloc_frame().ok_or("out of physical frames")?;
        self.map_user_page(page, frame, flags)?;
        Ok(frame)
    }
}
