//! `cat` — print file contents to stdout.
//!
//! [MANUAL] When FAT32 is functional, this will:
//! 1. `open(path, O_RDONLY)`
//! 2. Loop: `read(fd, buf, 512)` → `write(1, buf, n)`
//! 3. `close(fd)`

use crate::syscalls;

pub fn run(path: &[u8]) -> i32 {
    syscalls::print("cat: ");
    syscalls::print(core::str::from_utf8(path).unwrap_or("?"));
    syscalls::println("");

    // [MANUAL] When FAT32 read is available:
    // let fd = syscalls::open(path, 0);
    // if fd < 0 {
    //     syscalls::print("cat: no such file: ");
    //     syscalls::print(core::str::from_utf8(path).unwrap_or("?"));
    //     syscalls::println("");
    //     return 1;
    // }
    // let mut buf = [0u8; 512];
    // loop {
    //     let n = syscalls::read(fd as i32, &mut buf);
    //     if n <= 0 { break; }
    //     syscalls::write(1, &buf[..n as usize]);
    // }
    // syscalls::close(fd as i32);

    syscalls::println("  (FAT32 read not implemented — file contents unavailable)");
    0
}
