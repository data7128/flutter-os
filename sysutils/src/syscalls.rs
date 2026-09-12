//! Real syscall wrappers for sysutils — `int 0x80` on AeroOS.
//!
//! Mirrors the kernel syscall table (kernel/src/syscalls/mod.rs):
//! open=1 read=2 write=3 nanosleep=5 poll=9 kill=10 exit=11 exec=12
//! getpid=13 close=14 stat=17 fork=20 waitpid=21 getdents=23

pub const ENOSYS: i64 = -38;
pub const EINVAL: i64 = -22;
pub const EBADF: i64 = -9;
pub const ENOENT: i64 = -2;
pub const ENOTDIR: i64 = -20;
pub const EACCES: i64 = -13;

pub const SYS_OPEN: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_WRITE: u64 = 3;
pub const SYS_NANOSLEEP: u64 = 5;
pub const SYS_POLL_INPUT: u64 = 9;
pub const SYS_KILL: u64 = 10;
pub const SYS_EXIT: u64 = 11;
pub const SYS_EXEC: u64 = 12;
pub const SYS_GETPID: u64 = 13;
pub const SYS_CLOSE: u64 = 14;
pub const SYS_STAT: u64 = 17;
pub const SYS_FORK: u64 = 20;
pub const SYS_WAITPID: u64 = 21;
pub const SYS_GETDENTS: u64 = 23;

/// `stat` result, layout matches the kernel's `StatBuf`.
#[repr(C)]
pub struct Stat {
    pub st_size: u64,
    pub st_mode: u32,
}

/// getdents entry layout (72 bytes), matches the kernel:
/// [0] type (1=file, 2=dir, 3=char, 4=block), [1..65] name, rest zero.
pub const DENT_ENTRY_SIZE: usize = 72;

#[inline(never)]
unsafe fn syscall3(n: u64, a: u64, b: u64, c: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "int 0x80",
        inlateout("rax") n => ret,
        in("rdi") a,
        in("rsi") b,
        in("rdx") c,
        options(nostack)
    );
    ret
}

#[inline(never)]
unsafe fn syscall2(n: u64, a: u64, b: u64) -> i64 {
    syscall3(n, a, b, 0)
}

#[inline(never)]
unsafe fn syscall1(n: u64, a: u64) -> i64 {
    syscall3(n, a, 0, 0)
}

#[inline(never)]
unsafe fn syscall0(n: u64) -> i64 {
    syscall3(n, 0, 0, 0)
}

/// Write to stdout/stderr.
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    if buf.is_empty() {
        return 0;
    }
    unsafe { syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64) }
}

/// Read from fd (fd 0 = keyboard scancode buffer).
pub fn read(fd: i32, buf: &mut [u8]) -> i64 {
    unsafe { syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64) }
}

/// Open a file (flags: 0 = read-only for now).
/// Copy a byte slice into a NUL-terminated stack buffer (max 128 bytes).
/// The kernel reads paths as NUL-terminated C strings; passing a raw
/// `&[u8]` (Rust byte-string literals have no trailing NUL) makes the
/// kernel read garbage past the path.
fn path_cstr<'a>(path: &[u8], scratch: &'a mut [u8; 128]) -> Option<&'a [u8]> {
    if path.len() >= 128 {
        return None;
    }
    scratch[..path.len()].copy_from_slice(path);
    scratch[path.len()] = 0;
    Some(&scratch[..path.len() + 1])
}

pub fn open(path: &[u8], flags: u32) -> i64 {
    let mut scratch = [0u8; 128];
    let Some(p) = path_cstr(path, &mut scratch) else {
        return -28; // ENAMETOOLONG
    };
    unsafe { syscall3(SYS_OPEN, p.as_ptr() as u64, flags as u64, 0) }
}

/// Close a file descriptor.
pub fn close(fd: i32) -> i64 {
    unsafe { syscall1(SYS_CLOSE, fd as u64) }
}

/// List a directory: `getdents(path, buf, count)` returns bytes written
/// into `buf` (n * 72) or a negative errno.
pub fn getdents(path: &[u8], buf: *mut u8, count: usize) -> i64 {
    let mut scratch = [0u8; 128];
    let Some(p) = path_cstr(path, &mut scratch) else {
        return -28;
    };
    unsafe { syscall3(SYS_GETDENTS, p.as_ptr() as u64, buf as u64, count as u64) }
}

/// stat a path.
pub fn stat(path: &[u8], st: *mut Stat) -> i64 {
    let mut scratch = [0u8; 128];
    let Some(p) = path_cstr(path, &mut scratch) else {
        return -28;
    };
    unsafe { syscall3(SYS_STAT, p.as_ptr() as u64, st as u64, 0) }
}

/// Send a signal to a process.
pub fn kill(pid: u32, signum: u8) -> i64 {
    unsafe { syscall3(SYS_KILL, pid as u64, signum as u64, 0) }
}

/// Get the current PID.
pub fn getpid() -> i64 {
    unsafe { syscall0(SYS_GETPID) }
}

/// Nanosleep (ticks at 1 kHz).
pub fn nanosleep(ticks: u64) -> i64 {
    unsafe { syscall1(SYS_NANOSLEEP, ticks) }
}

/// Fork the current process. Returns 0 in the child, child PID in the
/// parent, negative errno on failure.
pub fn fork() -> i64 {
    unsafe { syscall0(SYS_FORK) }
}

/// Replace the current process image. `argv` is a null-terminated array
/// of string pointers, or null for an empty argv.
pub fn exec(path: &[u8], argv: *const *const u8) -> i64 {
    let mut scratch = [0u8; 128];
    let Some(p) = path_cstr(path, &mut scratch) else {
        return -28;
    };
    unsafe { syscall3(SYS_EXEC, p.as_ptr() as u64, argv as u64, 0) }
}

/// Wait for a child. `pid` = -1 (u64::MAX) waits for any child.
pub fn waitpid(pid: i64, status: *mut i32) -> i64 {
    unsafe { syscall3(SYS_WAITPID, pid as u64, status as u64, 0) }
}

/// Exit the process.
pub fn exit(status: i32) -> ! {
    unsafe {
        syscall1(SYS_EXIT, status as u64);
        core::arch::asm!("hlt", options(nomem, nostack));
    }
    loop {}
}

// ── helpers ───────────────────────────────────────────────────────────

/// Print a byte slice to stdout.
pub fn print(s: &[u8]) {
    write(1, s);
}

/// Print a string to stdout.
pub fn print_str(s: &str) {
    write(1, s.as_bytes());
}

/// Print a line to stdout.
pub fn println(s: &str) {
    write(1, s.as_bytes());
    write(1, b"\n");
}

/// Print an unsigned integer in decimal (no alloc needed).
pub fn print_u64(mut v: u64) {
    if v == 0 {
        write(1, b"0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 20;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    write(1, &buf[i..]);
}

/// Print a signed integer in decimal.
pub fn print_i64(v: i64) {
    if v < 0 {
        write(1, b"-");
        print_u64((-(v as i128)) as u64);
    } else {
        print_u64(v as u64);
    }
}
