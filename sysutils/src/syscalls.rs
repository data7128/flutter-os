//! Syscall wrappers for sysutils (mirror kernel syscalls).

pub const ENOSYS: i64 = -38;
pub const EINVAL: i64 = -22;
pub const EBADF: i64 = -9;
pub const ENOENT: i64 = -2;

/// Write to stdout/stderr.
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    let _ = (fd, buf);
    ENOSYS
}

/// Read from fd.
pub fn read(fd: i32, buf: &mut [u8]) -> i64 {
    let _ = (fd, buf);
    ENOSYS
}

/// Open a file.
pub fn open(path: &[u8], flags: u32) -> i64 {
    let _ = (path, flags);
    ENOSYS
}

/// Close a file descriptor.
pub fn close(fd: i32) -> i64 {
    let _ = fd;
    ENOSYS
}

/// Kill a process (send signal).
pub fn kill(pid: u32, signum: u8) -> i64 {
    let _ = (pid, signum);
    ENOSYS
}

/// Get current PID.
pub fn getpid() -> i64 {
    ENOSYS
}

/// Nanosleep.
pub fn nanosleep(ticks: u64) -> i64 {
    let _ = ticks;
    ENOSYS
}

/// Exit process.
pub fn exit(status: i32) -> ! {
    let _ = status;
    loop {
        nanosleep(1000);
    }
}

/// Print a string to stdout.
pub fn println(s: &str) {
    write(1, s.as_bytes());
    write(1, b"\n");
}

/// Print without newline.
pub fn print(s: &str) {
    write(1, s.as_bytes());
}
