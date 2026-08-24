//! Minimal POSIX-compatible syscall framework.
//!
//! Provides a syscall dispatch table via software interrupt (int 0x80).
//! Implements a minimal subset of POSIX syscalls needed for future
//! Flutter Engine user-mode support:
//!
//! - `open`  — open a file (skeleton: requires FAT32 driver)
//! - `read`  — read from fd (serial/framebuffer/stdin)
//! - `write` — write to fd (serial/VGA/framebuffer/stdout)
//! - `mmap`  — map memory (heap-backed)
//! - `nanosleep` — sleep by tick count
//! - `clock_gettime` — get kernel-maintained system time
//!
//! ## Not yet implemented (deferred to later stages)
//!
//! - `fork`   — requires process/task structs + context switching
//! - `exec`   — requires ELF loader + process address space management
//! - `signal` — requires signal delivery + signal mask per-process
//! - `pipe`, `socket`, `ioctl`, `epoll`, `poll`, `select`
//!
//! ## Design
//!
//! Syscalls are dispatched via `int 0x80` (vector 128). The syscall
//! number is in `rax`, arguments in `rdi, rsi, rdx, r10, r8, r9`.
//! Return value in `rax`. This mirrors the Linux 32-bit syscall ABI
//! (simplified). When Ring3 usermode is implemented, the `syscall`
//! instruction (MSR_STAR) will replace `int 0x80`.

pub mod fd;
pub mod time;

pub use fd::FdTable;

/// Syscall vector number for `int 0x80`.
pub const SYSCALL_VECTOR: u8 = 0x80;

/// POSIX-style error codes (negative return values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
#[allow(non_camel_case_types)]
pub enum Errno {
    ok = 0,
    ebadf = -9,
    einval = -22,
    enosys = -38,
    enoent = -2,
    enomem = -12,
    efault = -14,
}

impl Errno {
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Syscall numbers (must match user-mode header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SyscallNum {
    Open = 1,
    Read = 2,
    Write = 3,
    Mmap = 4,
    Nanosleep = 5,
    ClockGettime = 6,
    /// Query framebuffer info (address, width, height, stride, bpp, format).
    /// Returns a `FramebufferInfoUser` struct at the pointer in arg0.
    GetFramebufferInfo = 7,
}

impl SyscallNum {
    pub fn from_u64(n: u64) -> Option<Self> {
        match n {
            1 => Some(Self::Open),
            2 => Some(Self::Read),
            3 => Some(Self::Write),
            4 => Some(Self::Mmap),
            5 => Some(Self::Nanosleep),
            6 => Some(Self::ClockGettime),
            7 => Some(Self::GetFramebufferInfo),
            _ => None,
        }
    }
}

/// Maximum number of open file descriptors per process.
pub const MAX_FDS: usize = 64;

/// Global file descriptor table.
///
/// ← FUTURE: when per-process address spaces exist, each `Process` will
/// have its own `FdTable`. For now, a single global table suffices.
pub static FD_TABLE: spin::Mutex<FdTable> = spin::Mutex::new(FdTable::new());

/// Initialise the syscall subsystem.
///
/// Registers the syscall interrupt handler in the IDT and initialises
/// the FD table and time subsystem.
pub fn init() {
    // Register the syscall handler in the IDT.
    crate::interrupts::idt::set_syscall_handler();
    crate::serial::_print(format_args!("[syscalls] int 0x80 handler registered\n"));
}

/// Dispatch a syscall by number. Called from the interrupt handler
/// after extracting arguments from registers.
///
/// Returns `i64`: non-negative on success, negative errno on failure.
///
/// ← FUTURE: this is the function that Ring3 user-mode code will
/// invoke. Flutter Engine will call through this path.
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    num: u64,
    arg0: u64,
    arg1: u64,
    arg2: u64,
    _arg3: u64,
    _arg4: u64,
    _arg5: u64,
) -> i64 {
    // SAFETY: All syscall implementations operate on raw pointers
    // that originate from user-space. When Ring3 is implemented,
    // proper validation (copy_from_user, etc.) will be added here.
    unsafe {
        match SyscallNum::from_u64(num) {
            Some(SyscallNum::Open) => sys_open(arg0 as *const u8, arg1 as u32),
            Some(SyscallNum::Read) => sys_read(arg0 as i32, arg1 as *mut u8, arg2),
            Some(SyscallNum::Write) => sys_write(arg0 as i32, arg1 as *const u8, arg2),
            Some(SyscallNum::Mmap) => sys_mmap(arg0, arg1, arg2 as u32),
            Some(SyscallNum::Nanosleep) => sys_nanosleep(arg0, arg1),
            Some(SyscallNum::ClockGettime) => sys_clock_gettime(arg0, arg1),
            Some(SyscallNum::GetFramebufferInfo) => sys_get_framebuffer_info(arg0),
            None => Errno::enosys.as_i64(),
        }
    }
}

// ── Syscall implementations ──────────────────────────────────────────

/// `open(path, flags)` → fd number (≥0) or negative errno.
///
/// **Skeleton**: requires FAT32/ATA driver to be functional.
/// Currently returns `ENOSYS`.
///
/// ← FUTURE: when FAT32 is implemented, this will look up the file
/// on the ATA disk and allocate an FD entry.
unsafe fn sys_open(path: *const u8, _flags: u32) -> i64 {
    // TODO: resolve path through FAT32 filesystem.
    // TODO: allocate FD entry.
    //
    // SKELETON: log the path for debugging, return ENOSYS.
    if path.is_null() {
        return Errno::efault.as_i64();
    }
    crate::serial::_print(format_args!(
        "[syscall] open(\"{}\") — ENOSYS (no FAT32 driver yet)\n",
        unsafe { core::ffi::CStr::from_ptr(path as *const core::ffi::c_char) }
            .to_str()
            .unwrap_or("<invalid>")
    ));
    Errno::enosys.as_i64()
}

/// `read(fd, buf, count)` → bytes_read (≥0) or negative errno.
///
/// Currently supports:
/// - fd=0 (stdin): read from PS/2 keyboard scancode buffer (non-blocking)
///
/// ← FUTURE: support file-backed FDs from FAT32.
unsafe fn sys_read(fd: i32, buf: *mut u8, count: u64) -> i64 {
    if buf.is_null() {
        return Errno::efault.as_i64();
    }
    match fd {
        0 => {
            // stdin: drain PS/2 keyboard scancode buffer
            let mut read = 0u64;
            while read < count {
                if let Some(sc) = crate::interrupts::SCANCODE_BUFFER.lock().pop() {
                    *buf.add(read as usize) = sc;
                    read += 1;
                } else {
                    break;
                }
            }
            read as i64
        }
        _ => {
            // Check FD table for file-backed descriptors
            let table = FD_TABLE.lock();
            if table.get(fd as usize).is_some() {
                // TODO: read from FAT32 file via ATA driver
                Errno::enosys.as_i64()
            } else {
                Errno::ebadf.as_i64()
            }
        }
    }
}

/// `write(fd, buf, count)` → bytes_written (≥0) or negative errno.
///
/// Currently supports:
/// - fd=1 (stdout): write to serial + VGA
/// - fd=2 (stderr): write to serial only
///
/// ← FUTURE: support writing to framebuffer or files.
unsafe fn sys_write(fd: i32, buf: *const u8, count: u64) -> i64 {
    if buf.is_null() {
        return Errno::efault.as_i64();
    }
    let slice = core::slice::from_raw_parts(buf, count as usize);
    match fd {
        1 => {
            // stdout → serial + VGA
            crate::serial::_print(format_args!(
                "{}",
                core::str::from_utf8(slice).unwrap_or("<utf8 error>")
            ));
            crate::vga_buffer::_print(format_args!(
                "{}",
                core::str::from_utf8(slice).unwrap_or("<utf8 error>")
            ));
            count as i64
        }
        2 => {
            // stderr → serial only
            crate::serial::_print(format_args!(
                "{}",
                core::str::from_utf8(slice).unwrap_or("<utf8 error>")
            ));
            count as i64
        }
        _ => Errno::ebadf.as_i64(),
    }
}

/// `mmap(addr, len, prot)` → virtual address or negative errno.
///
/// Allocates memory from the kernel heap and returns the pointer.
/// This is a simplified mmap that ignores `MAP_FIXED`, file-backed
/// mapping, and protection flags (all memory is RWX in kernel mode).
///
/// ← FUTURE: when paging is implemented, this will allocate virtual
/// memory regions and map physical frames. Flutter Engine will use
/// mmap for texture upload, ELF loading, and shared memory.
unsafe fn sys_mmap(_addr: u64, len: u64, _prot: u32) -> i64 {
    use alloc::alloc::{alloc, Layout};
    if len == 0 {
        return Errno::einval.as_i64();
    }
    let layout = match Layout::from_size_align(len as usize, 4096) {
        Ok(l) => l,
        Err(_) => return Errno::einval.as_i64(),
    };
    let ptr = alloc(layout);
    if ptr.is_null() {
        return Errno::enomem.as_i64();
    }
    ptr as i64
}

/// `nanosleep(req_ticks, rem_ticks)` → 0 on success.
///
/// Sleeps for `req_ticks` PIT timer ticks (~1ms per tick at 1000Hz).
/// `rem_ticks` is not yet populated (always 0).
///
/// ← FUTURE: implement proper `rem` (remaining time if interrupted).
unsafe fn sys_nanosleep(req_ticks: u64, _rem: u64) -> i64 {
    let target = time::tick_count() + req_ticks;
    while time::tick_count() < target {
        x86_64::instructions::hlt();
    }
    0
}

/// `clock_gettime(clk_id, tp)` → 0 on success.
///
/// `tp` is a pointer to a `Timespec { tv_sec, tv_nsec }`.
/// Returns `CLOCK_MONOTONIC` (clk_id=1) based on PIT tick count.
///
/// ← FUTURE: implement `CLOCK_REALTIME` with RTC support.
unsafe fn sys_clock_gettime(clk_id: u64, tp: u64) -> i64 {
    if tp == 0 {
        return Errno::efault.as_i64();
    }
    // Only CLOCK_MONOTONIC (1) is supported.
    if clk_id != 1 {
        return Errno::einval.as_i64();
    }
    let (secs, nanos) = time::system_time();
    // Write Timespec struct: { i64 tv_sec; i64 tv_nsec; }
    let tp = tp as *mut i64;
    *tp = secs as i64;
    *tp.add(1) = nanos as i64;
    0
}

/// `get_framebuffer_info(info_ptr)` → 0 on success.
///
/// Writes a `FramebufferInfoUser` struct at `info_ptr` containing
/// the framebuffer address, dimensions, stride, bpp, and pixel format.
///
/// This lets user-mode code (flutter_adapter) know how to mmap the
/// framebuffer for rendering.
///
/// ← FUTURE: Flutter adapter calls this to obtain fb geometry before
/// calling mmap to map the fb into user space.
unsafe fn sys_get_framebuffer_info(info_ptr: u64) -> i64 {
    if info_ptr == 0 {
        return Errno::efault.as_i64();
    }

    let (fb_addr, fb_len, width, height, stride, bpp, format) = {
        match crate::graphics::get_fb_state() {
            Some(s) => s,
            None => return Errno::enosys.as_i64(),
        }
    };

    // Write FramebufferInfoUser struct: { u64 addr; u32 w; u32 h; u32 stride; u32 bpp; u32 fmt; }
    let ptr = info_ptr as *mut u64;
    *ptr = fb_addr as u64;
    *ptr.add(1) = width as u64;
    *ptr.add(2) = height as u64;
    *ptr.add(3) = stride as u64;
    *ptr.add(4) = bpp as u64;
    *ptr.add(5) = format as u64;
    *ptr.add(6) = fb_len as u64;

    0
}


