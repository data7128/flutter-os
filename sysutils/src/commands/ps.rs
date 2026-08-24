//! `ps` — list running processes.
//!
//! [MANUAL] When the process table is accessible via syscall, this
//! will read `/proc` or use a `SYS_LIST_PROCESSES` syscall to display
//! PID, state, name for each process.

use crate::syscalls;

pub fn run() -> i32 {
    syscalls::println("PID  STATE     NAME");
    syscalls::println("---  --------   ----");

    // [MANUAL] When Ring3 + process table syscall is available:
    // let nprocs = syscalls::list_processes(buf.as_mut_ptr(), buf.len());
    // for i in 0..nprocs {
    //     let p = &buf[i];
    //     syscalls::print(&format!("{:3}  {:8}  {}\n", p.pid, state_str, name));
    // }

    // Skeleton: show placeholder.
    syscalls::println("  1  Ready     init");
    syscalls::println("  2  Ready     wm");
    syscalls::println("  3  Ready     flutter_shell");
    syscalls::println("  (process table access requires SYS_LIST_PROCS)");
    0
}
