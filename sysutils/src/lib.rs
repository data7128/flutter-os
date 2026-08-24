//! AeroOS System Utilities — ls, cat, ps, kill.
//!
//! A busybox-style multi-call binary: the first argument determines
//! which utility to run. Designed as a Ring3 user-mode debug terminal.
//!
//! ## Usage
//! ```text
//! sysutils ls /          — list directory
//! sysutils cat /path     — print file contents
//! sysutils ps            — list processes
//! sysutils kill <pid>    — send SIGKILL to a process
//! ```
//!
//! ## Status
//! - Skeleton: all commands return ENOSYS (no FAT32/process table yet)
//! - [MANUAL] When Ring3 + FAT32 are ready, these will be functional

#![no_std]
#![allow(dead_code)]

pub mod syscalls;
pub mod commands;

/// Entry point — dispatch to the appropriate command.
///
/// `argv[0]` = program name ("sysutils" or the command name).
/// `argv[1]` = the command to run (ls/cat/ps/kill).
/// `argv[2+]` = command arguments.
pub fn main(argv: &[&[u8]]) -> i32 {
    if argv.len() < 2 {
        syscalls::println("AeroOS sysutils — usage: sysutils <command> [args]");
        syscalls::println("  ls <path>   — list directory");
        syscalls::println("  cat <path>  — print file");
        syscalls::println("  ps          — list processes");
        syscalls::println("  kill <pid>  — kill process");
        return 1;
    }

    let cmd = argv[1];

    if eq(cmd, b"ls") {
        commands::ls::run(argv.get(2).copied().unwrap_or(b"/"))
    } else if eq(cmd, b"cat") {
        if argv.len() < 3 {
            syscalls::println("cat: missing file path");
            return 1;
        }
        commands::cat::run(argv[2])
    } else if eq(cmd, b"ps") {
        commands::ps::run()
    } else if eq(cmd, b"kill") {
        if argv.len() < 3 {
            syscalls::println("kill: missing PID");
            return 1;
        }
        commands::kill::run(argv[2])
    } else {
        syscalls::print("sysutils: unknown command: ");
        syscalls::print(core::str::from_utf8(cmd).unwrap_or("?"));
        syscalls::println("");
        return 1;
    }
}

/// Compare two byte slices.
fn eq(a: &[u8], b: &[u8]) -> bool {
    a == b
}
