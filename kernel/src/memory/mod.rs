//! Kernel heap allocator.
//!
//! The bootloader maps the whole physical address space at
//! `physical_memory_offset` (when `map-physical-memory` is enabled in the
//! bootloader config). We pick the first large enough *Usable* region,
//! translate it to its virtual address (`region.start + offset`) and seed a
//! linked-list allocator over a 1 MiB slice. This gives the kernel a working
//! `alloc` without re-implementing paging from scratch.

use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use bootloader_api::BootInfo;
use linked_list_allocator::LockedHeap;

/// Size of the kernel heap.
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

/// Global allocator backed by a linked list of free blocks.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Set up the heap. Called once from `kernel_main`.
pub fn init(boot_info: &mut BootInfo) {
    let phys_offset = boot_info
        .physical_memory_offset
        .into_option()
        .expect("[memory] bootloader did not map physical memory (enable `map-physical-memory`)");

    let region = boot_info
        .memory_regions
        .iter()
        .find(|r: &&MemoryRegion| {
            r.kind == MemoryRegionKind::Usable && (r.end - r.start) >= HEAP_SIZE as u64
        })
        .expect("[memory] no usable memory region large enough for the heap");

    // The region is reported Usable by the firmware and is not used by the
    // bootloader, so using it as the heap backing store is sound.
    let heap_virt = region.start + phys_offset;

    unsafe {
        ALLOCATOR.lock().init(heap_virt as *mut u8, HEAP_SIZE);
    }
}
