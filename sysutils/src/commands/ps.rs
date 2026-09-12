//! `ps` — list running processes via the getprocs syscall.

use crate::syscalls;
use crate::syscalls::ProcInfo;

fn state_str(s: u64) -> &'static [u8] {
    match s {
        1 => b"Ready",
        2 => b"Running",
        3 => b"Blocked",
        4 => b"Zombie",
        _ => b"Unknown",
    }
}

pub fn run() -> i32 {
    let mut infos = [ProcInfo { pid: 0, ppid: 0, state: 0, name: [0u8; 16] }; 32];
    let n = syscalls::getprocs(infos.as_mut_ptr(), 32);
    if n < 0 {
        syscalls::print_str("ps: getprocs failed");
        syscalls::println("");
        return 1;
    }
    syscalls::println("PID   PPID  STATE    NAME");
    let mut i = 0usize;
    while (i as i64) < n {
        let p = &infos[i];
        syscalls::print_u64(p.pid);
        syscalls::print_str("  ");
        syscalls::print_u64(p.ppid);
        syscalls::print_str("  ");
        syscalls::print(state_str(p.state));
        // name: zero-padded 16 bytes
        let mut len = 0usize;
        while len < 16 && p.name[len] != 0 {
            len += 1;
        }
        syscalls::print_str("  ");
        syscalls::print(&p.name[..len]);
        syscalls::println("");
        i += 1;
    }
    0
}
