//! `ls` — list directory contents via the getdents syscall.
//!
//! Reads 72-byte entries from the kernel: [0] type, [1..65] name.

use crate::syscalls;

pub fn run(path: &[u8]) -> i32 {
    let mut buf = [0u8; 4096];
    let n = syscalls::getdents(path, buf.as_mut_ptr(), buf.len());
    if n < 0 {
        syscalls::print_str("ls: cannot open '");
        syscalls::print(path);
        syscalls::println("'");
        return 1;
    }
    if n == 0 {
        syscalls::print_str("ls: '");
        syscalls::print(path);
        syscalls::println("' is empty");
        return 0;
    }

    let mut i = 0usize;
    while i + syscalls::DENT_ENTRY_SIZE <= n as usize {
        let ty = buf[i];
        let name = &buf[i + 1..i + 65];
        // Name is null-terminated.
        let mut len = 0usize;
        while len < 64 && name[len] != 0 {
            len += 1;
        }
        let name_slice = &name[..len];
        match ty {
            2 => {
                syscalls::print_str("d  ");
                syscalls::print(name_slice);
            }
            1 => {
                syscalls::print_str("f  ");
                syscalls::print(name_slice);
            }
            3 => {
                syscalls::print_str("c  ");
                syscalls::print(name_slice);
            }
            4 => {
                syscalls::print_str("b  ");
                syscalls::print(name_slice);
            }
            _ => {
                syscalls::print_str("?  ");
                syscalls::print(name_slice);
            }
        }
        syscalls::println("");
        i += syscalls::DENT_ENTRY_SIZE;
    }
    0
}
