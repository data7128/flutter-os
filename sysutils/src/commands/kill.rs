//! `kill` — send a signal to a process.
//!
//! Usage: `kill <pid>` — sends SIGKILL (9) to the specified PID.
//! Usage: `kill -<signum> <pid>` — sends the specified signal.

use crate::syscalls;

const SIGKILL: u8 = 9;
const SIGTERM: u8 = 15;

pub fn run(arg: &[u8]) -> i32 {
    // Parse PID or signal+PID.
    let (signum, pid_str): (u8, &[u8]) = if arg.len() > 1 && arg[0] == b'-' {
        // Format: -<signum>
        let signum = parse_signum(&arg[1..]);
        (signum.unwrap_or(SIGKILL), &arg[1..])
    } else {
        (SIGKILL, arg)
    };

    let pid = match parse_u32(pid_str) {
        Some(p) => p,
        None => {
            // If we only got the signal, try to parse arg as PID directly.
            match parse_u32(arg) {
                Some(p) => p,
                None => {
                    syscalls::println("kill: invalid PID");
                    return 1;
                }
            }
        }
    };

    if pid == 0 {
        syscalls::println("kill: cannot kill PID 0 (kernel)");
        return 1;
    }

    let result = syscalls::kill(pid, signum);
    if result < 0 {
        syscalls::print("kill: failed to signal pid ");
        syscalls::print(core::str::from_utf8(arg).unwrap_or("?"));
        syscalls::println("");
        return 1;
    }

    syscalls::println("kill: signal sent");
    0
}

/// Parse a signal number from bytes (e.g. b"9" → 9).
fn parse_signum(s: &[u8]) -> Option<u8> {
    parse_u32(s).map(|v| v as u8)
}

/// Parse a u32 from decimal bytes.
fn parse_u32(s: &[u8]) -> Option<u32> {
    let mut result: u32 = 0;
    let mut any = false;
    for &b in s {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        any = true;
    }
    if any { Some(result) } else { None }
}
