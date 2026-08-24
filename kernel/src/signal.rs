//! Minimal signal mechanism — supports termination signals only.
//!
//! This is NOT a full POSIX signal implementation. It supports:
//! - `SIGKILL` (9) — unconditionally terminate a process
//! - `SIGTERM` (15) — request termination (same as SIGKILL in this skeleton)
//!
//! ## Not implemented (deferred)
//! - `SIGINT`, `SIGHUP`, `SIGSEGV`, `SIGALRM`, etc.
//! - Signal handlers (`sigaction`/`signal`)
//! - Signal blocking (`sigprocmask`)
//! - Signal delivery during syscall return
//! - Real-time signals
//!
//! [MANUAL] Full signal delivery requires:
//! - Ring3 → Ring0 → Ring3 signal frame injection
//! - User-mode signal handler invocation
//! - Signal-safe syscall restart

use crate::process::{ProcessState, PROCESS_TABLE};

/// Signal numbers (minimal subset).
pub const SIGKILL: u8 = 9;
pub const SIGTERM: u8 = 15;

/// Send a signal to a process.
///
/// For SIGKILL/SIGTERM: marks the process as Zombie and releases resources.
/// For other signals: sets the pending bit (not yet delivered).
///
/// Returns true if the signal was delivered, false if the process
/// doesn't exist or is already dead.
pub fn send_signal(pid: u32, signum: u8) -> bool {
    let mut table = PROCESS_TABLE.lock();

    // For termination signals, immediately mark the process as zombie.
    if signum == SIGKILL || signum == SIGTERM {
        if let Some(proc) = table.get_mut(pid) {
            if proc.state == ProcessState::Free || proc.state == ProcessState::Zombie {
                return false;
            }

            crate::serial::_print(format_args!(
                "[signal] killing pid={} signal={}\n", pid, signum
            ));

            // Release all resources.
            proc.fd_table.clear();
            proc.pending_signals = 0;
            proc.signal_mask = 0;
            proc.state = ProcessState::Zombie;
            proc.exit_code = 128 + (signum as i32);

            return true;
        }
        return false;
    }

    // For non-termination signals, just set the pending bit.
    table.send_signal(pid, signum)
}

/// Deliver pending signals to the currently running process.
///
/// Called from the timer interrupt handler before context switching.
///
/// [MANUAL] Full delivery requires injecting a signal frame on the
/// user-mode stack and jumping to the signal handler. For now,
/// we just process SIGKILL/SIGTERM (mark as zombie).
pub fn deliver_pending() {
    let mut table = PROCESS_TABLE.lock();
    let current = table.current_pid;

    if current == 0 {
        return; // Kernel mode, no signal delivery.
    }

    if let Some(proc) = table.get_mut(current) {
        let pending = proc.pending_signals;
        if pending == 0 {
            return;
        }

        // Check SIGKILL (bit 8).
        if (pending & (1 << (SIGKILL - 1))) != 0 {
            crate::serial::_print(format_args!(
                "[signal] delivering SIGKILL to pid={}\n", current
            ));
            proc.fd_table.clear();
            proc.pending_signals = 0;
            proc.state = ProcessState::Zombie;
            proc.exit_code = 128 + SIGKILL as i32;
            return;
        }

        // Check SIGTERM (bit 14).
        if (pending & (1 << (SIGTERM - 1))) != 0 {
            crate::serial::_print(format_args!(
                "[signal] delivering SIGTERM to pid={}\n", current
            ));
            proc.fd_table.clear();
            proc.pending_signals = 0;
            proc.state = ProcessState::Zombie;
            proc.exit_code = 128 + SIGTERM as i32;
            return;
        }

        // [MANUAL] Other signals need full signal frame delivery.
        // Clear pending non-termination signals (can't deliver yet).
        proc.pending_signals &= !((1 << (SIGKILL - 1)) | (1 << (SIGTERM - 1)));
    }
}

/// `kill(pid, signum)` → 0 on success, negative errno on failure.
pub fn sys_kill(pid: u32, signum: u8) -> i64 {
    if pid == 0 {
        return crate::syscalls::Errno::einval.as_i64();
    }
    if signum == 0 {
        // Signal 0 = existence check.
        let table = PROCESS_TABLE.lock();
        return if table.get(pid).is_some() { 0 } else { crate::syscalls::Errno::enoent.as_i64() };
    }
    if send_signal(pid, signum) {
        0
    } else {
        crate::syscalls::Errno::enoent.as_i64()
    }
}
