//! OOM (Out-Of-Memory) handler — graceful degradation on alloc failure.
//!
//! Instead of panicking immediately, the OOM handler:
//! 1. Tries to reclaim memory from zombie processes
//! 2. Tries to terminate the largest memory-consuming process
//! 3. Only panics if no recovery is possible
//!
//! [MANUAL] Full OOM requires:
//! - Per-process memory accounting (track pages allocated per process)
//! - Swap/page eviction to disk
//! - Memory pressure thresholds

use crate::process::{ProcessState, PROCESS_TABLE};

/// Called when allocation fails. Tries to recover before panicking.
///
/// Returns `true` if memory was reclaimed and the caller should retry,
/// `false` if no recovery is possible.
pub fn handle_oom(layout: &core::alloc::Layout) -> bool {
    crate::serial::_print(format_args!(
        "[oom] allocation failed: size={}, align={}\n",
        layout.size(), layout.align()
    ));

    // Step 1: Reap zombie processes to free their memory.
    {
        let mut table = PROCESS_TABLE.lock();
        let zombies_before = table.processes.iter()
            .filter(|p| p.state == ProcessState::Zombie)
            .count();

        if zombies_before > 0 {
            table.reap_zombies();
            crate::serial::_print(format_args!(
                "[oom] reaped {} zombie processes\n", zombies_before
            ));
            return true; // Retry allocation.
        }
    }

    // Step 2: No zombies to reap. Try terminating a Ready/Blocked process.
    //
    // In a real system, we'd pick the process with the largest memory
    // footprint. For now, we terminate the first Ready non-system process.
    {
        let mut table = PROCESS_TABLE.lock();
        for i in 0..table.processes.len() {
            let p = &table.processes[i];
            if p.pid > 2 && p.state == ProcessState::Ready {
                crate::serial::_print(format_args!(
                    "[oom] terminating pid={} to reclaim memory\n", p.pid
                ));
                table.processes[i].fd_table.clear();
                table.processes[i].state = ProcessState::Zombie;
                table.processes[i].exit_code = -9; // OOM kill
                table.reap_zombies();
                return true;
            }
        }
    }

    // Step 3: No process to kill. Fatal OOM.
    crate::serial::_print(format_args!(
        "[oom] FATAL: no memory could be reclaimed. Kernel will panic.\n"
    ));
    false
}
