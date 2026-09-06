//! Syscall trampoline + Ring3 user-mode entry.
//!
//! ## Syscall path (int 0x80)
//!
//! User code executes `int 0x80` with:
//! - `rax` = syscall number
//! - `rdi, rsi, rdx, r10, r8, r9` = arguments 0..5
//!
//! The CPU switches to Ring0 (using TSS.RSP0) and pushes
//! `SS, RSP, RFLAGS, CS, RIP`. Our trampoline then saves all
//! general-purpose registers, calls `syscall_dispatch` with a
//! pointer to the saved context, restores registers, and `iretq`s
//! back to Ring3. The return value is written to `ctx.rax`.
//!
//! ## Ring3 entry
//!
//! `enter_usermode(entry, user_stack_top)` constructs an artificial
//! interrupt stack frame and `iretq`s into Ring3 for the first time.

use core::arch::global_asm;

// ── Assembly trampoline ────────────────────────────────────────────────

global_asm!(
    r#"
.global syscall_trampoline
syscall_trampoline:
    // CPU already pushed: SS, RSP, RFLAGS, CS, RIP (from Ring3).
    // Save all GPRs so the Rust dispatcher can read args and write retval.
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    // First arg (rdi) = pointer to the saved context.
    mov rdi, rsp
    call syscall_dispatch

    // Restore all GPRs (rax now holds the return value).
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    // Return to Ring3: pops RIP, CS, RFLAGS, RSP, SS.
    iretq
"#
);

extern "C" {
    pub fn syscall_trampoline();
}

// ── Saved interrupt context ────────────────────────────────────────────

/// Full CPU context saved by `syscall_trampoline`.
///
/// Field order matches the push order in the assembly (top of stack = r15).
/// The five fields after `rax` are pushed by the CPU itself when `int 0x80`
/// transitions from Ring3 to Ring0.
#[repr(C)]
pub struct InterruptContext {
    // Pushed by trampoline (reverse push order):
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    // Pushed by CPU (int 0x80 from Ring3):
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Rust entry point called from the assembly trampoline.
///
/// Reads the syscall number from `ctx.rax` and arguments from
/// `ctx.rdi/rsi/rdx/r10/r8/r9`, dispatches, and writes the return
/// value back to `ctx.rax`.
#[no_mangle]
pub extern "C" fn syscall_dispatch(ctx: &mut InterruptContext) {
    let num = ctx.rax;
    let arg0 = ctx.rdi;
    let arg1 = ctx.rsi;
    let arg2 = ctx.rdx;
    let arg3 = ctx.r10;
    let arg4 = ctx.r8;
    let arg5 = ctx.r9;

    let retval = crate::syscalls::dispatch(num, arg0, arg1, arg2, arg3, arg4, arg5);
    ctx.rax = retval as u64;
}

// ── Ring3 entry ─────────────────────────────────────────────────────────

/// Enter Ring3 user mode for the first time.
///
/// Constructs an artificial interrupt stack frame on the kernel stack
/// and executes `iretq`, which pops `RIP, CS, RFLAGS, RSP, SS` and
/// lands in Ring3 at `entry` with `user_stack_top` as the user stack.
///
/// # Safety
/// - `entry` must be a valid user-mode executable address.
/// - `user_stack_top` must be a valid, mapped, writable page.
/// - The GDT must have user code/data segments loaded (see `gdt::init`).
/// - Interrupts must be enabled in RFLAGS (we set IF below).
pub unsafe fn enter_usermode(entry: u64, user_stack_top: u64) -> ! {
    let user_code_sel = crate::interrupts::gdt::user_code_selector();
    let user_data_sel = crate::interrupts::gdt::user_data_selector();

    // RFLAGS: IF (interrupt enable) bit set, IOPL=0.
    // Bit 1 (reserved, always 1) must also be set.
    let rflags: u64 = 0x200 | 0x2; // IF=1, reserved bit=1

    core::arch::asm!(
        // Push SS (user data selector with RPL=3).
        "push {ss}",
        // Push RSP (user stack top).
        "push {rsp}",
        // Push RFLAGS.
        "push {rflags}",
        // Push CS (user code selector with RPL=3).
        "push {cs}",
        // Push RIP (user entry point).
        "push {entry}",
        // Clear integer registers so we don't leak kernel state.
        "xor rax, rax",
        "xor rbx, rbx",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rsi, rsi",
        "xor rdi, rdi",
        "xor rbp, rbp",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",
        // Load user data selectors into DS, ES, FS, GS.
        "mov ds, {ds_sel:x}",
        "mov es, {ds_sel:x}",
        // iretq to Ring3.
        "iretq",
        ss = in(reg) user_data_sel.0 as u64,
        rsp = in(reg) user_stack_top,
        rflags = in(reg) rflags,
        cs = in(reg) user_code_sel.0 as u64,
        entry = in(reg) entry,
        ds_sel = in(reg) user_data_sel.0,
        options(noreturn),
    );
}
