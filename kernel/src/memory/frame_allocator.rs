//! Physical frame allocator.
//!
//! Uses the bootloader's memory map to track usable physical frames.
//! Implements the `x86_64` `FrameAllocator` trait so it can be used
//! with the page table mapper.
//!
//! Design: a simple stack of free physical frames. At init we walk
//! every usable memory region from the bootloader and push each 4 KiB
//! frame onto the stack. Allocation pops, deallocation pushes.

use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, PhysFrame, Size4KiB};
use x86_64::PhysAddr;

use spin::Mutex;

/// Maximum number of physical frames we can track.
/// 128 MiB / 4 KiB = 32768 frames — enough for QEMU and small machines.
const MAX_FRAMES: usize = 32768;

/// A simple stack-based physical frame allocator.
pub struct BootInfoFrameAllocator {
    free_stack: [u64; MAX_FRAMES],
    top: usize,
}

impl BootInfoFrameAllocator {
    /// Create an empty allocator. Must call `init` before use.
    pub const fn new() -> Self {
        Self {
            free_stack: [0; MAX_FRAMES],
            top: 0,
        }
    }

    /// Initialise the allocator from the bootloader memory map.
    ///
    /// Walks every usable region and pushes each 4 KiB-aligned frame
    /// onto the free stack. Frames already occupied by the kernel or
    /// bootloader are skipped (they are not marked Usable).
    pub fn init(&mut self, memory_regions: &'static mut [MemoryRegion]) {
        let mut count = 0usize;
        for region in memory_regions.iter() {
            if region.kind != MemoryRegionKind::Usable {
                continue;
            }
            let start = region.start;
            let end = region.end;
            // Align start up to 4 KiB.
            let frame_start = (start + 4095) & !4095;
            let mut addr = frame_start;
            while addr + 4096 <= end && count < MAX_FRAMES {
                self.free_stack[self.top] = addr;
                self.top += 1;
                count += 1;
                addr += 4096;
            }
            if count >= MAX_FRAMES {
                break;
            }
        }
        crate::serial::_print(format_args!(
            "[frame-alloc] initialised: {} free 4K frames ({} KiB)\n",
            self.top,
            self.top * 4
        ));
    }

    /// Allocate one physical frame. Returns the frame start address.
    ///
    /// The frame is zeroed before it is returned. This is critical for
    /// page-table frames: `x86_64` 0.14's `map_to` allocates intermediate
    /// page-table frames through this allocator and relies on them being
    /// clean — a reused frame with stale entries makes `map_to` report
    /// `PageAlreadyMapped` for a brand-new virtual page.
    pub fn allocate_frame(&mut self) -> Option<PhysAddr> {
        if self.top == 0 {
            return None;
        }
        self.top -= 1;
        let pa = self.free_stack[self.top];
        let po = *crate::memory::page_table::PHYSICAL_OFFSET.lock();
        unsafe {
            core::ptr::write_bytes((po + pa) as *mut u8, 0, 4096);
        }
        Some(PhysAddr::new(pa))
    }

    /// Deallocate a physical frame.
    pub fn deallocate_frame(&mut self, frame: PhysAddr) {
        if self.top >= MAX_FRAMES {
            return; // leak if stack full
        }
        self.free_stack[self.top] = frame.as_u64();
        self.top += 1;
    }

    /// Number of free frames.
    pub fn free_count(&self) -> usize {
        self.top
    }
}

/// Global frame allocator instance.
pub static FRAME_ALLOCATOR: Mutex<BootInfoFrameAllocator> =
    Mutex::new(BootInfoFrameAllocator::new());

// ── x86_64 FrameAllocator trait impls ────────────────────────────────

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.allocate_frame()
            .map(|addr| PhysFrame::from_start_address(addr).unwrap())
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        self.deallocate_frame(frame.start_address());
    }
}

/// Convenience: allocate a frame through the global allocator.
pub fn alloc_frame() -> Option<PhysFrame<Size4KiB>> {
    FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .map(|addr| PhysFrame::from_start_address(addr).unwrap())
}

/// Convenience: deallocate a frame through the global allocator.
pub fn dealloc_frame(frame: PhysFrame<Size4KiB>) {
    FRAME_ALLOCATOR.lock().deallocate_frame(frame.start_address());
}
