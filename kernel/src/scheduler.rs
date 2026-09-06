//! Preemptive round-robin scheduler.
//!
//! ## Design
//!
//! Each runnable process has a kernel stack and a saved context. The
//! timer interrupt (PIT, ~1 kHz) calls `on_timer_tick`, which decrements
//! the current process's time slice. When the slice reaches zero,
//! `schedule()` picks the next runnable process and performs a context
//! switch via the assembly `switch_context`.
//!
//! ## Context switch
//!
//! `switch_context(old_rsp: &mut u64, new_rsp: u64)`:
//! 1. Push callee-saved registers (rbx, rbp, r12–r15) onto the current stack.
//! 2. Save the current RSP into `*old_rsp`.
//! 3. Load RSP from `new_rsp`.
//! 4. Pop callee-saved registers from the new stack.
//! 5. `ret` — pops the return address that was pushed when the new
//!    process was descheduled (or the initial `entry` for a fresh process).
//!
//! Because context switches happen inside the timer interrupt handler,
//! caller-saved registers are already preserved by the x86-interrupt ABI.

use core::arch::global_asm;
use spin::Mutex;

use crate::process::{ProcessState, PROCESS_TABLE};

// ── Assembly: context switch ───────────────────────────────────────────

global_asm!(
    r#"
.global switch_context
switch_context:
    // rdi = &mut old_rsp, rsi = new_rsp
    // Save callee-saved registers on the current (old) kernel stack.
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15

    // Save old stack pointer.
    mov [rdi], rsp

    // Switch to new stack.
    mov rsp, rsi

    // Restore callee-saved registers from the new stack.
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp

    // Return to the new process's saved instruction pointer.
    ret
"#
);

extern "C" {
    /// Switch from the current kernel stack to `new_rsp`.
    ///
    /// # Safety
    /// `old_rsp` must point to a valid `u64` that receives the old RSP.
    /// `new_rsp` must point to a valid kernel stack with a saved context
    /// (callee-saved registers + return address) laid out by `switch_context`.
    fn switch_context(old_rsp: *mut u64, new_rsp: u64);
}

// ── Per-process kernel stack ────────────────────────────────────────────

/// Size of the per-process kernel stack (used when the process is in Ring0).
pub const PROCESS_KERNEL_STACK_SIZE: usize = 32 * 1024; // 32 KiB

/// A process's kernel stack + saved RSP.
pub struct ProcessKernelStack {
    /// Stack memory (statically allocated per process slot).
    pub stack: [u8; PROCESS_KERNEL_STACK_SIZE],
    /// Saved kernel RSP when the process is descheduled.
    pub saved_rsp: u64,
    /// Whether the initial context has been set up.
    pub initialised: bool,
}

impl ProcessKernelStack {
    pub const fn new() -> Self {
        Self {
            stack: [0; PROCESS_KERNEL_STACK_SIZE],
            saved_rsp: 0,
            initialised: false,
        }
    }

    /// Top of the stack (stacks grow downward).
    pub fn stack_top(&self) -> u64 {
        let bottom = self.stack.as_ptr() as u64;
        bottom + PROCESS_KERNEL_STACK_SIZE as u64
    }

    /// Set up the initial context for a fresh process.
    ///
    /// Lays out the stack so that `switch_context` will "return" to
    /// `entry_point` in Ring0. The entry function should then perform
    /// the `iretq` to Ring3.
    ///
    /// Stack layout (from high to low address):
    /// ```text
    ///   stack_top →  ...
    ///                 entry_point   (pushed as "return address")
    ///                 r15 = 0
    ///                 r14 = 0
    ///                 r13 = 0
    ///                 r12 = 0
    ///                 rbx = 0
    ///                 rbp = 0
    ///   saved_rsp  →  (points here)
    /// ```
    pub fn setup_initial_context(&mut self, entry_point: u64) {
        let top = self.stack_top();
        // We push 7 values (entry + 6 callee-saved regs) = 56 bytes.
        let rsp = top - 56;

        unsafe {
            let ptr = rsp as *mut u64;
            // Callee-saved registers (popped in order: r15, r14, ..., rbp).
            *ptr.add(0) = 0; // r15
            *ptr.add(1) = 0; // r14
            *ptr.add(2) = 0; // r13
            *ptr.add(3) = 0; // r12
            *ptr.add(4) = 0; // rbx
            *ptr.add(5) = 0; // rbp
            // Return address → entry_point.
            *ptr.add(6) = entry_point;
        }

        self.saved_rsp = rsp;
        self.initialised = true;
    }
}

// ── Global scheduler state ──────────────────────────────────────────────

/// Per-process kernel stacks, indexed by process slot.
///
/// We use a fixed array matching `MAX_PROCESSES` so each process slot
/// has a dedicated kernel stack.
pub static KERNEL_STACKS: Mutex<[ProcessKernelStack; 32]> =
    Mutex::new([const { ProcessKernelStack::new() }; 32]);

/// Scheduler state.
pub struct Scheduler {
    /// Time slice remaining for the current process (in timer ticks).
    pub current_slice: u32,
    /// Default time slice (ticks). ~5ms at 1kHz PIT.
    pub default_slice: u32,
    /// Whether the scheduler is active (has started scheduling user procs).
    pub active: bool,
}

pub static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler {
    current_slice: 5,
    default_slice: 5,
    active: false,
});

/// Find the slot index for a given PID.
fn find_slot(pid: u32) -> Option<usize> {
    let table = PROCESS_TABLE.lock();
    table.processes.iter().position(|p| p.pid == pid)
}

/// Called on every timer tick. Decrements the current time slice and
/// triggers a reschedule when it reaches zero.
pub fn on_timer_tick() {
    let mut sched = SCHEDULER.lock();
    if !sched.active {
        return;
    }
    if sched.current_slice > 0 {
        sched.current_slice -= 1;
    }
    if sched.current_slice == 0 {
        drop(sched);
        schedule();
    }
}

/// Pick the next runnable process and switch to it.
///
/// Round-robin: scan from the current PID forward, wrap around, and
/// pick the first process in `Ready` state.
pub fn schedule() {
    let (current_pid, next_pid) = {
        let table = PROCESS_TABLE.lock();
        let current = table.current_pid;
        let current_slot = table
            .processes
            .iter()
            .position(|p| p.pid == current)
            .unwrap_or(0);

        // Scan forward from current slot, wrap around.
        let mut next: Option<u32> = None;
        for i in 1..=32 {
            let idx = (current_slot + i) % 32;
            let p = &table.processes[idx];
            if p.pid != 0 && p.state == ProcessState::Ready {
                next = Some(p.pid);
                break;
            }
        }
        (current, next)
    };

    let next_pid = match next_pid {
        Some(pid) => pid,
        None => return, // no other runnable process
    };

    if next_pid == current_pid {
        // Reset slice and continue.
        SCHEDULER.lock().current_slice = SCHEDULER.lock().default_slice;
        return;
    }

    // Perform the context switch.
    do_context_switch(current_pid, next_pid);
}

/// Perform a context switch from `old_pid` to `new_pid`.
fn do_context_switch(old_pid: u32, new_pid: u32) {
    let old_slot = match find_slot(old_pid) {
        Some(s) => s,
        None => return,
    };
    let new_slot = match find_slot(new_pid) {
        Some(s) => s,
        None => return,
    };

    let mut stacks = KERNEL_STACKS.lock();

    // Ensure the new process has an initial context.
    if !stacks[new_slot].initialised {
        // The entry point for a fresh process is `trampoline_to_user`,
        // which does the iretq to Ring3. We set it up when the process
        // is created, but if not, skip.
        crate::serial::_print(format_args!(
            "[sched] new process pid={} has no initial context — skipping\n",
            new_pid
        ));
        return;
    }

    let new_rsp = stacks[new_slot].saved_rsp;
    let old_rsp_ptr = &mut stacks[old_slot].saved_rsp as *mut u64;

    // Update process states.
    {
        let mut table = PROCESS_TABLE.lock();
        if let Some(p) = table.get_mut(old_pid) {
            if p.state == ProcessState::Running {
                p.state = ProcessState::Ready;
            }
        }
        if let Some(p) = table.get_mut(new_pid) {
            p.state = ProcessState::Running;
        }
        table.current_pid = new_pid;
    }

    // Reset time slice.
    SCHEDULER.lock().current_slice = SCHEDULER.lock().default_slice;

    // Switch TSS.RSP0 to the new process's kernel stack top.
    // This ensures that if the new process triggers an interrupt/syscall
    // from Ring3, the CPU uses the correct kernel stack.
    unsafe {
        set_tss_rsp0(stacks[new_slot].stack_top());
    }

    // Perform the actual register/stack switch.
    unsafe {
        switch_context(old_rsp_ptr, new_rsp);
    }
}

/// Update TSS.RSP0 (the kernel stack used for Ring3→Ring0 transitions).
///
/// # Safety
/// Writes to the TSS, which is referenced by the GDT. The address must
/// be a valid, mapped kernel stack top.
unsafe fn set_tss_rsp0(rsp0: u64) {
    // We access the TSS through the x86_64 crate's TaskStateSegment.
    // The TSS is a lazy_static; we mutate its privilege_stack_table[0].
    //
    // Since the TSS is behind a lazy_static (which uses a Once), and we
    // only ever write RSP0 from the scheduler (with interrupts disabled
    // during context switch), this is safe in practice.
    let tss_ptr = {
        // tss_address() forces the lazy_static TSS to initialise and
        // returns its virtual address.
        crate::interrupts::gdt::tss_address()
    };

    if !tss_ptr.is_null() {
        // RSP0 is at offset 4 in the TSS (after the reserved u32 at offset 0).
        let rsp0_byte_ptr = (tss_ptr as *mut u8).add(4) as *mut u64;
        *rsp0_byte_ptr = rsp0;
    }
}

/// Start the scheduler. Called after at least one user process is ready.
pub fn start_scheduler() {
    SCHEDULER.lock().active = true;
    crate::serial::_print(format_args!("[sched] scheduler started\n"));
}

/// Initialise a process's kernel stack with the given Ring0 entry point.
///
/// The entry point should be a function that sets up user segments and
/// performs `iretq` to Ring3 (see `syscall_trampoline::enter_usermode`).
pub fn init_process_stack(slot: usize, entry_point: u64) {
    let mut stacks = KERNEL_STACKS.lock();
    stacks[slot].setup_initial_context(entry_point);
}
