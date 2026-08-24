//! Global Descriptor Table (GDT) + Task State Segment (TSS).
//!
//! The GDT defines the kernel code segment. The TSS provides an
//! independent interrupt stack table so that a double fault cannot
//! corrupt the currently-in-use stack.

use lazy_static::lazy_static;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// Index into the TSS interrupt stack table used for the double fault.
pub const DOUBLE_FAULT_IST: u16 = 0;

/// 20 KiB, 16-byte aligned private stack for double faults.
#[repr(C, align(16))]
struct DoubleFaultStack([u8; 20480]);

static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack([0; 20480]);

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST as usize] = {
            // Top of the stack = bottom + size (stacks grow downwards).
            let ptr = core::ptr::addr_of_mut!(DOUBLE_FAULT_STACK);
            let bottom = ptr as usize;
            VirtAddr::new((bottom + core::mem::size_of::<DoubleFaultStack>()) as u64)
        };
        tss
    };

    static ref GDT: (GlobalDescriptorTable, SegmentSelector, SegmentSelector, SegmentSelector) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(&*TSS));
        (gdt, code, data, tss)
    };
}

/// Load the GDT, refresh code + data segments and install the TSS.
pub fn init() {
    use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
    use x86_64::instructions::tables::load_tss;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1);
        SS::set_reg(GDT.2);
        DS::set_reg(GDT.2);
        ES::set_reg(GDT.2);
        load_tss(GDT.3);
    }
}
