//! `ls` — list directory contents.
//!
//! [MANUAL] When FAT32 is functional, this will:
//! 1. `open(path, O_RDONLY)` to get a directory FD
//! 2. Read directory entries via `getdents`-like syscall
//! 3. Print each entry name

use crate::syscalls;

pub fn run(path: &[u8]) -> i32 {
    syscalls::print("ls: ");
    syscalls::print(core::str::from_utf8(path).unwrap_or("?"));
    syscalls::println("");

    // [MANUAL] When FAT32 read is available:
    // let fd = syscalls::open(path, 0);
    // if fd < 0 {
    //     syscalls::print("ls: cannot open: ");
    //     syscalls::print(core::str::from_utf8(path).unwrap_or("?"));
    //     syscalls::println("");
    //     return 1;
    // }
    // ... read directory entries ...

    syscalls::println("  (no files — FAT32 read not implemented)");
    0
}
