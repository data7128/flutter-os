//! Process table — minimal process management for Ring3.
//!
//! Each process has a PID, page table (future), register state
//! (saved context), file descriptor table, and signal state.
//!
//! [MANUAL] Context switching (actual `iretq` to Ring3) requires:
//! - TSS with RSP0 for privilege-level switch
//! - User-mode stack allocation
//! - User code/data segment selectors in GDT
//! - Page table isolation
//!
//! ## Status
//! - Skeleton: process structs and table management ✅
//! - Ring3 context switch: 【必须人工开发，AI无法完整生成】

use crate::syscalls::FdTable;
use spin::Mutex;

/// Maximum number of concurrent processes.
pub const MAX_PROCESSES: usize = 32;

/// Process states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProcessState {
    /// Slot is free.
    Free = 0,
    /// Process is runnable.
    Ready = 1,
    /// Currently running.
    Running = 2,
    /// Waiting for I/O or sleep.
    Blocked = 3,
    /// Process has exited, pending cleanup.
    Zombie = 4,
}

/// Signal number (minimal subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SignalNum {
    /// Terminate (default action: terminate).
    Sigkill = 9,
    /// Terminate (can be caught).
    Sigterm = 15,
}

/// Saved CPU context for a process.
///
/// This is the register state saved on the kernel stack when
/// a process is preempted or blocks.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SavedContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl Default for SavedContext {
    fn default() -> Self {
        Self {
            rax: 0, rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0,
            rbp: 0, rsp: 0, r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0, rip: 0, rflags: 0,
        }
    }
}

impl SavedContext {
    pub const fn zero() -> Self {
        Self {
            rax: 0, rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0,
            rbp: 0, rsp: 0, r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0, rip: 0, rflags: 0,
        }
    }
}

/// A single process descriptor.
#[derive(Clone, Copy)]
pub struct Process {
    /// Process ID (0 = empty slot).
    pub pid: u32,
    /// Parent PID (0 = kernel).
    pub parent_pid: u32,
    /// Process state.
    pub state: ProcessState,
    /// Saved CPU context.
    pub context: SavedContext,
    /// Entry point (ELF entry address).
    pub entry_point: u64,
    /// User-mode stack pointer.
    pub user_rsp: u64,
    /// Physical address of this process's PML4 (CR3 value). 0 = inherit
    /// the kernel address space (not yet spawned from an ELF).
    pub cr3: u64,
    /// File descriptor table.
    pub fd_table: FdTable,
    /// Pending signals (bitmask: bit N = signal N+1 pending).
    pub pending_signals: u32,
    /// Signal mask (blocked signals).
    pub signal_mask: u32,
    /// Exit code (if Zombie).
    pub exit_code: i32,
    /// Process name (for ps display).
    pub name: [u8; 16],
}

impl Process {
    pub const fn empty() -> Self {
        Self {
            pid: 0,
            parent_pid: 0,
            state: ProcessState::Free,
            context: SavedContext::zero(),
            entry_point: 0,
            user_rsp: 0,
            cr3: 0,
            fd_table: FdTable::new(),
            pending_signals: 0,
            signal_mask: 0,
            exit_code: 0,
            name: [0; 16],
        }
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let len = name.len().min(15);
        self.name[..len].copy_from_slice(&name[..len]);
        self.name[len] = 0;
    }
}

/// Global process table.
pub static PROCESS_TABLE: Mutex<ProcessTable> = Mutex::new(ProcessTable::new());

/// Fixed-capacity process table.
pub struct ProcessTable {
    pub(crate) processes: [Process; MAX_PROCESSES],
    next_pid: u32,
    /// Currently running process PID (0 = kernel).
    pub current_pid: u32,
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            processes: [Process::empty(); MAX_PROCESSES],
            next_pid: 1,
            current_pid: 0,
        }
    }

    /// Allocate a new process. Returns PID or 0 on failure.
    pub fn alloc(&mut self, parent_pid: u32, name: &[u8]) -> u32 {
        for i in 0..MAX_PROCESSES {
            if self.processes[i].pid == 0 {
                let pid = self.next_pid;
                self.next_pid += 1;
                self.processes[i] = Process {
                    pid,
                    parent_pid,
                    // Fresh slots are not runnable until a spawn path
                    // (exec / spawn_user_process) sets up their kernel
                    // stack and flips the state to Ready. Keeping them
                    // Blocked prevents the scheduler from selecting
                    // placeholders (e.g. boot_service stubs) forever.
                    state: ProcessState::Blocked,
                    ..Process::empty()
                };
                self.processes[i].set_name(name);
                return pid;
            }
        }
        0
    }

    /// Free a process slot (mark as Free). The process's user address
    /// space (CR3) is deallocated first so its frames become reusable.
    pub fn free(&mut self, pid: u32) -> bool {
        for i in 0..MAX_PROCESSES {
            if self.processes[i].pid == pid {
                dealloc_proc_space(self.processes[i].cr3);
                self.processes[i].fd_table.clear();
                self.processes[i] = Process::empty();
                return true;
            }
        }
        false
    }

    /// Get a process by PID.
    pub fn get(&self, pid: u32) -> Option<&Process> {
        self.processes.iter().find(|p| p.pid == pid)
    }

    /// Get a mutable process by PID.
    pub fn get_mut(&mut self, pid: u32) -> Option<&mut Process> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }

    /// Send a signal to a process.
    pub fn send_signal(&mut self, pid: u32, signum: u8) -> bool {
        if let Some(proc) = self.get_mut(pid) {
            if proc.state == ProcessState::Free {
                return false;
            }
            // Set the pending signal bit.
            proc.pending_signals |= 1 << (signum - 1);
            return true;
        }
        false
    }

    /// Mark a process as exited.
    pub fn mark_exit(&mut self, pid: u32, exit_code: i32) -> bool {
        if let Some(proc) = self.get_mut(pid) {
            proc.state = ProcessState::Zombie;
            proc.exit_code = exit_code;
            return true;
        }
        false
    }

    /// Reap zombie processes (free their slots).
    pub fn reap_zombies(&mut self) {
        for i in 0..MAX_PROCESSES {
            if self.processes[i].state == ProcessState::Zombie {
                dealloc_proc_space(self.processes[i].cr3);
                // Release FD table resources.
                self.processes[i].fd_table.clear();
                self.processes[i] = Process::empty();
            }
        }
    }
}

/// Deallocate a process's user address space (frames below PML4 index 1).
/// CR3 == 0 means the process shares the kernel space — nothing to free.
fn dealloc_proc_space(cr3: u64) {
    if cr3 == 0 {
        return;
    }
    use x86_64::structures::paging::{PhysFrame, Size4KiB};
    use x86_64::PhysAddr;
    if let Ok(frame) = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(cr3)) {
        crate::memory::page_table::AddressSpace { pml4_frame: frame }.dealloc_user();
    }
}
