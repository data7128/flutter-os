//! Interrupt Descriptor Table (IDT): CPU exceptions + hardware IRQs + syscall.
//!
//! Routes every interrupt vector to a handler. Wires up:
//! - CPU exceptions: breakpoint, double fault, page fault, general protection
//! - Hardware IRQs: timer, keyboard, mouse (via 8259 PIC)
//! - Syscall: `int 0x80` at vector 128, DPL=3 (user-accessible)

use lazy_static::lazy_static;
use x86_64::instructions::port::Port;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};
use x86_64::PrivilegeLevel;

use crate::interrupts::{InterruptIndex, PICS, SCANCODE_BUFFER};
use crate::interrupts::mouse::MOUSE_STATE;
use crate::println;
use crate::syscall_trampoline::syscall_trampoline;

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        // ── CPU exceptions ──────────────────────────────────────────
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::interrupts::gdt::DOUBLE_FAULT_IST);
        }
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_handler);
        idt.stack_segment_fault
            .set_handler_fn(stack_segment_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.divide_error.set_handler_fn(divide_error_handler);

        // ── Hardware IRQs (8259 PIC) ───────────────────────────────
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(mouse_interrupt_handler);
        // IDE channel interrupts (kept masked; handler is a safety net).
        idt[InterruptIndex::IdePrimary.as_u8()].set_handler_fn(ide_interrupt_handler);
        idt[InterruptIndex::IdeSecondary.as_u8()].set_handler_fn(ide_interrupt_handler);

        // ── Syscall (int 0x80, DPL=3) ──────────────────────────────
        // The trampoline is a naked assembly function; transmute its
        // address to the x86-interrupt handler type so the IDT entry
        // gets the right offset. The CPU doesn't care about the Rust
        // ABI — it just jumps to the offset.
        let syscall_handler: extern "x86-interrupt" fn(InterruptStackFrame) =
            unsafe { core::mem::transmute(syscall_trampoline as *const () as usize) };
        idt[0x80]
            .set_handler_fn(syscall_handler)
            .set_privilege_level(PrivilegeLevel::Ring3);

        idt
    };
}

/// Load the IDT into the CPU (`lidt`).
pub fn init() {
    IDT.load();
    crate::serial::_print(format_args!("[idt] loaded (syscall 0x80 DPL=3)\n"));
}

/// Legacy: no-op kept for API compatibility. Syscall handler is now
/// installed statically in the `IDT` lazy_static above.
pub fn set_syscall_handler() {
    // Already registered in lazy_static IDT with DPL=3.
}

// ── CPU exception handlers ─────────────────────────────────────────────

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("[int] BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    panic!("[int] DOUBLE FAULT (code {})\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let accessed_addr = Cr2::read();
    println!(
        "[int] PAGE FAULT at addr={:#?}, error={:?}\n{:#?}",
        accessed_addr, error_code, stack_frame
    );

    // If the fault came from user mode (CS RPL = 3), deliver SIGSEGV.
    let cs = stack_frame.code_segment;
    if cs.rpl() == x86_64::PrivilegeLevel::Ring3 {
        println!("[int] page fault in user mode — killing current process");
        let pid = crate::process::PROCESS_TABLE.lock().current_pid;
        crate::process::PROCESS_TABLE.lock().mark_exit(pid, 139); // SIGSEGV = 139
        // TODO: switch to next runnable process via scheduler.
        crate::hlt_loop();
    }

    panic!("unrecoverable kernel page fault");
}

extern "x86-interrupt" fn general_protection_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    println!(
        "[int] GENERAL PROTECTION FAULT (code {})\n{:#?}",
        error_code, stack_frame
    );
    panic!("general protection fault");
}

extern "x86-interrupt" fn stack_segment_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    println!(
        "[int] STACK SEGMENT FAULT (code {})\n{:#?}",
        error_code, stack_frame
    );
    panic!("stack segment fault");
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    println!("[int] INVALID OPCODE\n{:#?}", stack_frame);
    panic!("invalid opcode");
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    println!("[int] DIVIDE ERROR\n{:#?}", stack_frame);
    panic!("divide error");
}

// ── Hardware IRQ handlers ──────────────────────────────────────────────

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Increment the kernel tick counter.
    crate::syscalls::time::on_timer_tick();

    // Preemptive scheduling: the scheduler's on_timer_tick decrements
    // the current process's time slice and may trigger a context switch.
    crate::scheduler::on_timer_tick();

    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    SCANCODE_BUFFER.lock().push(scancode);

    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let mut port = Port::new(0x60);
    let data: u8 = unsafe { port.read() };
    MOUSE_STATE.lock().on_byte(data);

    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
    }
}

/// Defensive handler for IDE channel interrupts. The ATA driver polls
/// (PIO) and the lines are kept masked, but if an IDE IRQ ever arrives
/// we must ack it instead of leaving the PIC waiting forever.
extern "x86-interrupt" fn ide_interrupt_handler(_stack_frame: InterruptStackFrame) {
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::IdePrimary.as_u8());
    }
}
