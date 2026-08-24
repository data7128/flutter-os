//! User-mode syscall wrappers — thin wrappers around `int 0x80`.
//!
//! These functions invoke the kernel's syscall dispatch table via the
//! `int 0x80` software interrupt. The calling convention mirrors the
//! simplified Linux ABI:
//!
//! - `rax` = syscall number
//! - `rdi` = arg0, `rsi` = arg1, `rdx` = arg2
//! - Return value in `rax` (non-negative = success, negative = errno)
//!
//! [MANUAL] When Ring3 is implemented, these will use inline asm to
//! execute `int 0x80`. Currently they are stubs that return ENOSYS.

/// Syscall numbers — must match kernel `syscalls::SyscallNum`.
pub const SYS_OPEN: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_WRITE: u64 = 3;
pub const SYS_MMAP: u64 = 4;
pub const SYS_NANOSLEEP: u64 = 5;
pub const SYS_CLOCK_GETTIME: u64 = 6;
/// [FUTURE] New syscall to query framebuffer info (address, width, height).
pub const SYS_GET_FB_INFO: u64 = 7;

/// Framebuffer info returned by `SYS_GET_FB_INFO`.
///
/// Laid out as a C struct so it can be written by the kernel into
/// user-provided memory.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    /// Physical framebuffer address (or virtual if mapped).
    pub address: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Stride (pixels per row, may be > width).
    pub stride: u32,
    /// Bytes per pixel (3 = BGR, 4 = RGBX).
    pub bytes_per_pixel: u32,
    /// Pixel format (0 = RGB, 1 = BGR, 2 = other).
    pub format: u32,
}

/// Errno values — must match kernel `Errno`.
pub const ENOSYS: i64 = -38;
pub const EBADF: i64 = -9;
pub const EINVAL: i64 = -22;
pub const ENOMEM: i64 = -12;
pub const EFAULT: i64 = -14;

/// Initialise the syscall interface.
///
/// Verifies that `clock_gettime` works — if not, the kernel doesn't
/// have the syscall framework ready.
pub fn init() -> Result<(), i32> {
    let mut ts = [0i64; 2];
    let ret = clock_gettime(1, &mut ts as *mut i64 as u64); // CLOCK_MONOTONIC
    if ret < 0 {
        return Err(ret as i32);
    }
    Ok(())
}

/// `write(fd, buf, count)` — write bytes to a file descriptor.
///
/// fd=1 → stdout (serial + VGA)
/// fd=2 → stderr (serial only)
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    // [MANUAL] When Ring3 is ready, replace with:
    //   unsafe { asm!("int 0x80",
    //     in("rax") SYS_WRITE,
    //     in("rdi") fd, in("rsi") buf.as_ptr(), in("rdx") buf.len(),
    //     out("rax") ret, ...) }
    //
    // SKELETON: cannot issue int 0x80 from Ring0, so this is a no-op.
    let _ = (fd, buf);
    ENOSYS
}

/// `read(fd, buf, count)` — read bytes from a file descriptor.
///
/// fd=0 → stdin (PS/2 keyboard scancodes)
pub fn read(fd: i32, buf: &mut [u8]) -> i64 {
    let _ = (fd, buf);
    ENOSYS
}

/// `mmap(addr, len, prot)` — allocate/map memory.
pub fn mmap(addr: u64, len: u64, prot: u32) -> i64 {
    let _ = (addr, len, prot);
    ENOSYS
}

/// `nanosleep(ticks)` — sleep for N PIT ticks.
pub fn nanosleep(ticks: u64) -> i64 {
    let _ = ticks;
    ENOSYS
}

/// `clock_gettime(clk_id, tp)` — get system time.
///
/// tp points to a `Timespec { tv_sec: i64, tv_nsec: i64 }`.
pub fn clock_gettime(clk_id: u64, tp: u64) -> i64 {
    let _ = (clk_id, tp);
    ENOSYS
}

/// `get_framebuffer_info(info_ptr)` — query framebuffer geometry.
///
/// [MANUAL] This syscall is not yet implemented in the kernel.
/// When ready, the kernel will write a `FramebufferInfo` struct at
/// the provided address.
pub fn get_framebuffer_info(info: &mut FramebufferInfo) -> i64 {
    // [MANUAL] syscall(SYS_GET_FB_INFO, info as *mut _ as u64)
    let _ = info;
    ENOSYS
}
