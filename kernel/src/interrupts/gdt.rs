//! Global Descriptor Table (GDT) + Task State Segment (TSS).
//!
//! The GDT defines kernel and user code/data segments plus the TSS.
//! The TSS provides:
//! - `RSP0`: the kernel stack used when transitioning from Ring3 → Ring0
//!   (syscall / interrupt / exception).
//! - `IST[0]`: a private stack for double faults so a corrupted stack
//!   cannot prevent the double-fault handler from running.
//!
//! Segment selectors (byte offsets into the GDT):
//! - 0x08 = kernel code
//! - 0x10 = kernel data
//! - 0x18 = TSS
//! - 0x20 = user data  (DPL=3, RPL=3 → selector 0x23)
//! - 0x28 = user code  (DPL=3, RPL=3 → selector 0x2B)

use lazy_static::lazy_static;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// Index into the TSS interrupt stack table used for the double fault.
pub const DOUBLE_FAULT_IST: u16 = 0;

/// Size of the kernel stack (used as RSP0 for Ring3→Ring0 transitions).
pub const KERNEL_STACK_SIZE: usize = 64 * 1024; // 64 KiB

/// 20 KiB, 16-byte aligned private stack for double faults.
#[repr(C, align(16))]
struct DoubleFaultStack([u8; 20480]);

static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack([0; 20480]);

/// Kernel stack for Ring0 execution when entering from Ring3.
#[repr(C, align(16))]
struct KernelStack([u8; KERNEL_STACK_SIZE]);

static mut KERNEL_STACK: KernelStack = KernelStack([0; KERNEL_STACK_SIZE]);

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();

        // RSP0: kernel stack top (stacks grow downward).
        tss.privilege_stack_table[0] = {
            let ptr = core::ptr::addr_of_mut!(KERNEL_STACK);
            let bottom = ptr as usize;
            VirtAddr::new((bottom + KERNEL_STACK_SIZE) as u64)
        };

        // IST[0]: double-fault private stack.
        tss.interrupt_stack_table[DOUBLE_FAULT_IST as usize] = {
            let ptr = core::ptr::addr_of_mut!(DOUBLE_FAULT_STACK);
            let bottom = ptr as usize;
            VirtAddr::new((bottom + core::mem::size_of::<DoubleFaultStack>()) as u64)
        };

        tss
    };

    static ref GDT: (
        GlobalDescriptorTable,
        SegmentSelector, // kernel code
        SegmentSelector, // kernel data
        SegmentSelector, // tss
        SegmentSelector, // user data
        SegmentSelector, // user code
    ) = {
        let mut gdt = GlobalDescriptorTable::new();
        let kernel_code = gdt.append(Descriptor::kernel_code_segment());
        let kernel_data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(&*TSS));
        let user_data = gdt.append(Descriptor::user_data_segment());
        let user_code = gdt.append(Descriptor::user_code_segment());
        (gdt, kernel_code, kernel_data, tss, user_data, user_code)
    };
}

/// User-mode code segment selector (with RPL=3).
pub fn user_code_selector() -> SegmentSelector {
    GDT.5
}

/// User-mode data segment selector (with RPL=3).
pub fn user_data_selector() -> SegmentSelector {
    GDT.4
}

/// Kernel code segment selector.
pub fn kernel_code_selector() -> SegmentSelector {
    GDT.1
}

/// Kernel data segment selector.
pub fn kernel_data_selector() -> SegmentSelector {
    GDT.2
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
    crate::serial::_print(format_args!(
        "[gdt] user code sel={:#x}, user data sel={:#x}, RSP0 set\n",
        GDT.5.index(),
        GDT.4.index()
    ));
}

/// Get the virtual address of the TSS.
///
/// Used by the scheduler to update `RSP0` when switching processes.
/// The TSS is behind a `lazy_static`; this forces initialisation and
/// returns a raw pointer to it.
pub fn tss_address() -> *mut u8 {
    let tss_ref: &TaskStateSegment = &*TSS;
    tss_ref as *const TaskStateSegment as *mut u8
}
