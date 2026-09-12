//! `cat` — print file contents to stdout.

use crate::syscalls;

pub fn run(path: &[u8]) -> i32 {
    let fd = syscalls::open(path, 0);
    if fd < 0 {
        syscalls::print_str("cat: no such file: ");
        syscalls::print(path);
        syscalls::println("");
        return 1;
    }

    let mut buf = [0u8; 512];
    loop {
        let n = syscalls::read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        let w = syscalls::write(1, &buf[..n as usize]);
        if w < 0 {
            break;
        }
    }
    syscalls::close(fd as i32);
    0
}
