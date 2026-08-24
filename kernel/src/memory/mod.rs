//! Kernel heap allocator.
//!
//! Uses a static byte array as the heap backing store. This avoids the need
//! for the bootloader to map all physical memory (which would conflict with
//! the kernel's own virtual address space). The 1 MiB heap is more than
//! sufficient for a minimal kernel.

use linked_list_allocator::LockedHeap;

/// Size of the kernel heap.
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

/// Static backing store for the heap. Linked into the kernel's BSS section.
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// Global allocator backed by a linked list of free blocks.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Set up the heap. Called once from `kernel_main`.
///
/// Uses a static array as the backing store, so no BootInfo or physical
/// memory mapping is required.
pub fn init() {
    // SAFETY: `HEAP` is a static array that exists for the entire lifetime of
    // the kernel. `init` is called exactly once from `kernel_main` before any
    // allocation occurs. The `LockedHeap` implementation ensures thread-safe
    // access via a spinlock.
    unsafe {
        let heap_start = core::ptr::addr_of_mut!(HEAP) as *mut u8;
        ALLOCATOR.lock().init(heap_start, HEAP_SIZE);
    }
}
