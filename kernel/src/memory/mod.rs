//! Kernel memory management.
//!
//! Submodules:
//! - `frame_allocator` — physical 4 KiB frame allocator (bootloader memory map)
//! - `page_table` — x86_64 4-level paging, kernel + user address spaces
//!
//! The heap allocator (`LockedHeap` over a static 1 MiB array) is kept
//! for early-boot allocations before the frame allocator is online.

pub mod frame_allocator;
pub mod page_table;

use linked_list_allocator::LockedHeap;

/// Size of the kernel heap.
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

/// Static backing store for the heap.
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// Global allocator backed by a linked list of free blocks.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Early heap init (called before frame allocator / paging).
pub fn init() {
    unsafe {
        let heap_start = core::ptr::addr_of_mut!(HEAP) as *mut u8;
        ALLOCATOR.lock().init(heap_start, HEAP_SIZE);
    }
}

/// Full memory subsystem init: frame allocator + kernel page table.
///
/// Called from `kernel_main` after the bootloader's `BootInfo` is
/// available. The heap is already online by this point.
///
/// # Safety
/// `physical_memory_offset` must be the value supplied by the bootloader.
pub unsafe fn init_memory_subsystem(
    memory_regions: &'static mut [bootloader_api::info::MemoryRegion],
    physical_memory_offset: u64,
) {
    // 1. Physical frame allocator
    frame_allocator::FRAME_ALLOCATOR.lock().init(memory_regions);

    // 2. Kernel page table (OffsetPageTable over the bootloader's PML4)
    page_table::init(x86_64::VirtAddr::new(physical_memory_offset));

    crate::println!("[OK] MEMORY_SUBSYSTEM");
}
