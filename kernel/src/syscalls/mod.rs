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
pub mod input;
pub mod time;

pub use fd::FdKind;
pub use fd::FdTable;
use crate::userproc::USER_STACK_TOP;
use alloc::borrow::ToOwned;

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
    /// Commit (blit) a rendered buffer to the physical framebuffer.
    /// arg0 = buffer pointer, arg1 = x, arg2 = y, arg3 = width, arg4 = height.
    FbCommit = 8,
    /// Poll for the next input event (keyboard or mouse).
    /// arg0 = pointer to `InputEvent` struct to fill.
    /// Returns 1 if event available, 0 if none.
    PollInput = 9,
    /// `kill(pid, signum)` — send a signal to a process.
    /// arg0 = pid, arg1 = signum.
    Kill = 10,
    /// `exit(status)` — terminate the calling process.
    /// arg0 = exit code.
    Exit = 11,
    /// `exec(path, argv)` — load and run an ELF executable.
    /// arg0 = path pointer, arg1 = argv pointer (unused in skeleton).
    Exec = 12,
    /// `getpid()` — return the current process ID.
    Getpid = 13,
    /// `close(fd)` — close a file descriptor.
    Close = 14,
    /// `mkdir(path)` — create a directory.
    Mkdir = 15,
    /// `unlink(path)` — remove a file.
    Unlink = 16,
    /// `stat(path, buf)` — get file metadata.
    Stat = 17,
    /// `chdir(path)` — change current directory.
    Chdir = 18,
    /// `getcwd(buf, size)` — get current working directory.
    Getcwd = 19,
    /// `fork()` — duplicate the calling process. Returns 0 in the child,
    /// the child's PID in the parent.
    Fork = 20,
    /// `waitpid(pid, status_ptr)` — wait for a child to exit and reap it.
    /// pid=-1 waits for any child. Returns the child PID.
    Waitpid = 21,
    /// `getppid()` — return the parent process ID.
    Getppid = 22,
    /// `getdents(path, buf, count)` — list a directory.
    /// arg0 = path pointer, arg1 = user buffer, arg2 = buffer size.
    /// Returns bytes written (n * 72) or a negative errno.
    Getdents = 23,
    /// `getprocs(buf, count)` — fill up to `count` ProcInfo entries
    /// (40 bytes each: pid u64, ppid u64, state u64, name[16]) from the
    /// process table. Returns the number of entries written.
    Getprocs = 24,
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
            8 => Some(Self::FbCommit),
            9 => Some(Self::PollInput),
            10 => Some(Self::Kill),
            11 => Some(Self::Exit),
            12 => Some(Self::Exec),
            13 => Some(Self::Getpid),
            14 => Some(Self::Close),
            15 => Some(Self::Mkdir),
            16 => Some(Self::Unlink),
            17 => Some(Self::Stat),
            18 => Some(Self::Chdir),
            19 => Some(Self::Getcwd),
            20 => Some(Self::Fork),
            21 => Some(Self::Waitpid),
            22 => Some(Self::Getppid),
            23 => Some(Self::Getdents),
            24 => Some(Self::Getprocs),
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

/// EPERM error code (operation not permitted).
const EPERM: i64 = -1;

/// Access the calling process's file descriptor table.
///
/// Every process owns its own `FdTable` (inherited by children at fork).
/// When the kernel itself issues a syscall (current_pid == 0), fall back
/// to the legacy global table.
fn with_fd_table_mut<T>(f: impl FnOnce(&mut FdTable) -> T) -> T {
    let pid = crate::process::PROCESS_TABLE.lock().current_pid;
    if pid != 0 {
        let mut table = crate::process::PROCESS_TABLE.lock();
        if let Some(proc) = table.get_mut(pid) {
            return f(&mut proc.fd_table);
        }
    }
    let mut global = FD_TABLE.lock();
    f(&mut global)
}

/// Check if the current process has permission for a resource.
/// Returns `true` if access is granted, `false` if denied.
///
/// When Ring3 is implemented, this will be called before executing
/// sensitive syscalls (fb_commit, poll_input, kill, exec, etc.).
fn check_perm(resource: crate::perm::Resource) -> bool {
    let pid = crate::process::PROCESS_TABLE.lock().current_pid;
    crate::perm::check_permission(pid, resource)
}

/// Dispatch a syscall by number. Called from the interrupt handler
/// after extracting arguments from registers.
///
/// Returns `i64`: non-negative on success, negative errno on failure.
///
/// Permission checks are applied to sensitive syscalls:
/// - `fb_commit` / `get_framebuffer_info` → Resource::Framebuffer
/// - `poll_input` → Resource::InputEvents
/// - `kill` → Resource::ProcessControl
/// - `exec` → Resource::Exec
///
/// ← FUTURE: this is the function that Ring3 user-mode code will
/// invoke. Flutter Engine will call through this path.
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    num: u64,
    arg0: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    _arg4: u64,
    _arg5: u64,
    ctx: *mut crate::syscall_trampoline::InterruptContext,
) -> i64 {
    // Pre-dispatch permission checks for sensitive syscalls.
    // In Ring 0 mode (current_pid=0) all checks pass; in Ring 3
    // user processes will be filtered by privilege level.
    let perm_resource = match SyscallNum::from_u64(num) {
        Some(SyscallNum::GetFramebufferInfo) => Some(crate::perm::Resource::Framebuffer),
        Some(SyscallNum::FbCommit) => Some(crate::perm::Resource::Framebuffer),
        Some(SyscallNum::PollInput) => Some(crate::perm::Resource::InputEvents),
        Some(SyscallNum::Kill) => Some(crate::perm::Resource::ProcessControl),
        Some(SyscallNum::Exec) => Some(crate::perm::Resource::Exec),
        _ => None,
    };
    if let Some(res) = perm_resource {
        if !check_perm(res) {
            crate::serial::_print(format_args!(
                "[syscall] EPERM: pid lacks {:?} permission\n", res
            ));
            return EPERM;
        }
    }

    unsafe {
        match SyscallNum::from_u64(num) {
            Some(SyscallNum::Open) => sys_open(arg0 as *const u8, arg1 as u32),
            Some(SyscallNum::Read) => sys_read(arg0 as i32, arg1 as *mut u8, arg2),
            Some(SyscallNum::Write) => sys_write(arg0 as i32, arg1 as *const u8, arg2),
            Some(SyscallNum::Mmap) => sys_mmap(arg0, arg1, arg2 as u32),
            Some(SyscallNum::Nanosleep) => sys_nanosleep(arg0, arg1),
            Some(SyscallNum::ClockGettime) => sys_clock_gettime(arg0, arg1),
            Some(SyscallNum::GetFramebufferInfo) => sys_get_framebuffer_info(arg0),
            Some(SyscallNum::FbCommit) => sys_fb_commit(arg0, arg1, arg2, arg3, _arg4),
            Some(SyscallNum::PollInput) => sys_poll_input(arg0),
            Some(SyscallNum::Kill) => sys_kill(arg0 as u32, arg1 as u8),
            Some(SyscallNum::Exit) => sys_exit(arg0 as i32),
            Some(SyscallNum::Exec) => sys_exec(arg0 as *const u8, arg1 as *const u64),
            Some(SyscallNum::Getdents) => sys_getdents(arg0 as *const u8, arg1 as *mut u8, arg2 as u64),
            Some(SyscallNum::Getprocs) => sys_getprocs(arg0 as *mut u8, arg1),
            Some(SyscallNum::Getpid) => sys_getpid(),
            Some(SyscallNum::Close) => sys_close(arg0 as i32),
            Some(SyscallNum::Mkdir) => sys_mkdir(arg0 as *const u8),
            Some(SyscallNum::Unlink) => sys_unlink(arg0 as *const u8),
            Some(SyscallNum::Stat) => sys_stat(arg0 as *const u8, arg1 as *mut StatBuf),
            Some(SyscallNum::Chdir) => sys_chdir(arg0 as *const u8),
            Some(SyscallNum::Getcwd) => sys_getcwd(arg0 as *mut u8, arg1),
            Some(SyscallNum::Fork) => sys_fork(ctx),
            Some(SyscallNum::Waitpid) => sys_waitpid(arg0, arg1),
            Some(SyscallNum::Getppid) => sys_getppid(),
            None => Errno::enosys.as_i64(),
        }
    }
}

// ── Syscall implementations ──────────────────────────────────────────

/// `open(path, flags)` → fd number (≥0) or negative errno.
///
/// Opens a file through the VFS (currently tmpfs-backed). Supports
/// O_CREAT (bit 0x40) to create a new file if it doesn't exist.
unsafe fn sys_open(path: *const u8, flags: u32) -> i64 {
    if path.is_null() {
        return Errno::efault.as_i64();
    }
    // Read null-terminated path.
    let mut len = 0usize;
    while *path.add(len) != 0 {
        len += 1;
        if len > 255 { return Errno::einval.as_i64(); }
    }
    let path_slice = core::slice::from_raw_parts(path, len);
    let path_str = match core::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return Errno::einval.as_i64(),
    };

    let create = (flags & 0x40) != 0; // O_CREAT
    let (mount_idx, inode_id) = {
        let vfs = crate::fs::VFS.lock();
        let file = match vfs.open(path_str, create) {
            Ok(f) => f,
            Err(e) => {
                crate::serial::_print(format_args!("[syscall] open(\"{}\") failed: {}\n", path_str, e));
                return Errno::enoent.as_i64();
            }
        };
        (file.mount_idx, file.inode_id)
    };

    let fd = with_fd_table_mut(|table| {
        table.alloc(crate::syscalls::fd::FdKind::VfsFile {
            mount_idx,
            inode_id,
            offset: 0,
        })
    });
    match fd {
        Some(f) => f as i64,
        None => Errno::enomem.as_i64(),
    }
}

/// `read(fd, buf, count)` → bytes_read (≥0) or negative errno.
///
/// Currently supports:
/// - fd=0 (stdin): blocking read from the COM1 serial console (the
///   host terminal under `qemu -serial stdio`).
///
/// ← FUTURE: support file-backed FDs from FAT32.
unsafe fn sys_read(fd: i32, buf: *mut u8, count: u64) -> i64 {
    if buf.is_null() {
        return Errno::efault.as_i64();
    }
    match fd {
        0 => {
            // stdin: block until at least one byte arrives, then drain
            // whatever else is already buffered. Yields so other
            // processes keep running while we wait for input.
            let mut read = 0u64;
            loop {
                if let Some(b) = crate::serial::try_read_byte() {
                    *buf.add(read as usize) = b;
                    read += 1;
                    break;
                }
                crate::scheduler::yield_now();
            }
            while read < count {
                match crate::serial::try_read_byte() {
                    Some(b) => {
                        *buf.add(read as usize) = b;
                        read += 1;
                    }
                    None => break,
                }
            }
            read as i64
        }
        _ => {
            // Check the process FD table for file-backed descriptors.
            let kind = with_fd_table_mut(|table| table.get(fd as usize).map(|e| e.kind));
            match kind {
                Some(FdKind::VfsFile { mount_idx, inode_id, offset }) => {
                    // Read from VFS file.
                    let vfs = crate::fs::VFS.lock();
                    let mut vfs_file = crate::fs::VfsFile {
                        inode_id, mount_idx, offset, file_type: crate::fs::FileType::Regular,
                    };
                    let n = vfs.read(&mut vfs_file, core::slice::from_raw_parts_mut(buf, count as usize));
                    let new_offset = vfs_file.offset;
                    drop(vfs);
                    // Update offset in the process FD table.
                    with_fd_table_mut(|t| t.update_vfs_offset(fd as usize, new_offset));
                    match n {
                        Ok(bytes) => bytes as i64,
                        Err(_) => Errno::einval.as_i64(),
                    }
                }
                Some(_) => {
                    // Legacy File kind — not yet implemented.
                    Errno::enosys.as_i64()
                }
                None => Errno::ebadf.as_i64(),
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
        _ => {
            // VFS file write through the process FD table.
            let kind = with_fd_table_mut(|table| table.get(fd as usize).map(|e| e.kind));
            match kind {
                Some(FdKind::VfsFile { mount_idx, inode_id, offset }) => {
                    let vfs = crate::fs::VFS.lock();
                    let mut vfs_file = crate::fs::VfsFile {
                        inode_id, mount_idx, offset, file_type: crate::fs::FileType::Regular,
                    };
                    let n = vfs.write(&mut vfs_file, slice);
                    let new_offset = vfs_file.offset;
                    drop(vfs);
                    with_fd_table_mut(|t| t.update_vfs_offset(fd as usize, new_offset));
                    match n {
                        Ok(bytes) => bytes as i64,
                        Err(_) => Errno::einval.as_i64(),
                    }
                }
                Some(_) => Errno::ebadf.as_i64(),
                None => Errno::ebadf.as_i64(),
            }
        }
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

/// `fb_commit(buf, x, y, w, h)` → 0 on success.
///
/// Copies a rectangular region from the user-provided buffer to the
/// physical framebuffer. The buffer must be in the framebuffer's
/// native pixel format (matching bpp).
///
/// This is the **double-buffer commit** path: the WM renders to an
/// off-screen buffer, then calls this syscall to blit it to the screen.
///
/// Args:
/// - `buf_ptr` (arg0): pointer to the source pixel buffer
/// - `x` (arg1): destination X offset on the framebuffer
/// - `y` (arg2): destination Y offset on the framebuffer
/// - `w` (arg3): width of the region in pixels
/// - `h` (arg4): height of the region in pixels
unsafe fn sys_fb_commit(buf_ptr: u64, x: u64, y: u64, w: u64, h: u64) -> i64 {
    if buf_ptr == 0 {
        return Errno::efault.as_i64();
    }
    if w == 0 || h == 0 {
        return Errno::einval.as_i64();
    }

    let (fb_addr, fb_len, fb_width, fb_height, fb_stride, bpp, _format) =
        match crate::graphics::get_fb_state() {
            Some(s) => s,
            None => return Errno::enosys.as_i64(),
        };

    // Validate bounds.
    if x >= fb_width as u64 || y >= fb_height as u64 {
        return Errno::einval.as_i64();
    }
    let x_end = x.checked_add(w).unwrap_or(fb_width as u64);
    let y_end = y.checked_add(h).unwrap_or(fb_height as u64);
    if x_end > fb_width as u64 || y_end > fb_height as u64 {
        return Errno::einval.as_i64();
    }

    let src = buf_ptr as *const u8;
    let dst = fb_addr as *mut u8;
    let bpp = bpp as usize;
    let stride = fb_stride as usize;
    let row_bytes = w as usize * bpp;

    for row in 0..h as usize {
        let src_off = row * row_bytes;
        let dst_off = ((y as usize + row) * stride + x as usize) * bpp;
        let copy_len = row_bytes;

        if dst_off + copy_len > fb_len {
            break;
        }

        core::ptr::copy_nonoverlapping(
            src.add(src_off),
            dst.add(dst_off),
            copy_len,
        );
    }

    0
}

/// `poll_input(event_ptr)` → 1 if event available, 0 if none.
///
/// Fills an `InputEvent` struct at the user-provided pointer with the
/// next available keyboard or mouse event.
///
/// The WM calls this in its event loop to get structured input events.
/// The kernel handles PS/2 scancode → InputEvent conversion and
/// mouse packet → delta conversion internally.
unsafe fn sys_poll_input(event_ptr: u64) -> i64 {
    if event_ptr == 0 {
        return Errno::efault.as_i64();
    }

    let event = event_ptr as *mut input::InputEvent;
    let mut ev = input::InputEvent::default();
    let ret = input::poll_next(&mut ev);

    if ret > 0 {
        core::ptr::write_volatile(event, ev);
    }

    ret
}

/// `kill(pid, signum)` → 0 on success, negative errno on failure.
unsafe fn sys_kill(pid: u32, signum: u8) -> i64 {
    crate::signal::sys_kill(pid, signum)
}

/// `exit(status)` → never returns (marks process as zombie).
///
/// Marks the calling process as a zombie, then hands control to the
/// scheduler so the next runnable process is switched in. If no other
/// process is runnable the kernel idles (the timer interrupt will
/// reschedule later).
unsafe fn sys_exit(status: i32) -> ! {
    let current = crate::process::PROCESS_TABLE.lock().current_pid;
    crate::serial::_print(format_args!(
        "[syscall] exit({}) from pid={}\n", status, current
    ));

    // Release resources and mark as zombie.
    crate::process::PROCESS_TABLE.lock().mark_exit(current, status);

    // Switch to the next runnable process (never returns if one exists).
    crate::scheduler::switch_to_next();
    crate::hlt_loop();
}

/// `exec(path)` → 0 on success, negative errno on failure.
///
/// Loads an ELF executable and prepares a new process.
unsafe fn sys_exec(path: *const u8, argv: *const u64) -> i64 {
    if path.is_null() {
        return Errno::efault.as_i64();
    }

    // Read the null-terminated path.
    let mut len = 0usize;
    while *path.add(len) != 0 {
        len += 1;
        if len > 255 {
            return Errno::einval.as_i64(); // Path too long.
        }
    }
    let path_slice = core::slice::from_raw_parts(path, len);

    // Security validation: path must start with '/'.
    if path_slice.is_empty() || path_slice[0] != b'/' {
        crate::serial::_print(format_args!("[exec] rejected: path must be absolute\n"));
        return Errno::einval.as_i64();
    }

    // Security validation: reject path traversal (..).
    let path_str = core::str::from_utf8(path_slice).unwrap_or("");
    if path_str.contains("..") {
        crate::serial::_print(format_args!("[exec] rejected: path traversal detected\n"));
        return Errno::einval.as_i64();
    }

    // Read the new ELF image.
    let elf_data = match crate::fs::VFS.lock().read_all(path_str) {
        Ok(d) => d,
        Err(_) => {
            crate::serial::_print(format_args!("[exec] cannot read \"{}\"\n", path_str));
            return Errno::enoent.as_i64();
        }
    };
    if elf_data.is_empty() {
        return Errno::enoent.as_i64();
    }

    // Load the ELF into a fresh address space (real image replacement:
    // the new image occupies the SAME process/PID, not a new one).
    let (addr_space, entry, _stack_top) =
        match crate::userproc::load_elf_into_new_space(&elf_data) {
            Ok(v) => v,
            Err(e) => {
                crate::serial::_print(format_args!(
                    "[exec] failed to load \"{}\": {}\n",
                    path_str, e
                ));
                return Errno::enoent.as_i64();
            }
        };
    let new_cr3 = addr_space.pml4_frame.start_address().as_u64();

    // Collect argv (up to 16 args, each ≤ 64 bytes, total ≤ 512 bytes)
    // from the OLD user address space (still active at this point).
    let mut argv_strs: [&[u8]; 16] = [b""; 16];
    let mut argc: usize = 0;
    let mut total_argv = 0usize;
    if !argv.is_null() {
        let mut i = 0usize;
        while i < 16 {
            let p = *argv.add(i);
            if p == 0 {
                break;
            }
            let mut l = 0usize;
            while *(p as *const u8).add(l) != 0 && l < 64 {
                l += 1;
            }
            if l == 0 {
                break;
            }
            if total_argv + l + 1 > 512 {
                break;
            }
            argv_strs[i] = core::slice::from_raw_parts(p as *const u8, l);
            total_argv += l + 1;
            argc += 1;
            i += 1;
        }
    }

    // Push argv/argc onto the NEW user stack (top page of the new space).
    let stack_top_phys = match addr_space.translate(USER_STACK_TOP - 1) {
        Some(p) => p,
        None => return Errno::efault.as_i64(),
    };
    let phys_offset = *crate::memory::page_table::PHYSICAL_OFFSET.lock();
    let stack_page_virt = phys_offset + stack_top_phys.as_u64();
    let page_base_virt = USER_STACK_TOP - 4096;
    let mut sp = stack_page_virt + 4096; // top of the stack page

    let mut ptrs: [u64; 16] = [0; 16];
    for i in (0..argc).rev() {
        let arg = argv_strs[i];
        sp -= (arg.len() + 1) as u64;
        core::ptr::copy_nonoverlapping(arg.as_ptr(), sp as *mut u8, arg.len());
        *(sp as *mut u8).add(arg.len()) = 0;
        ptrs[i] = page_base_virt + (sp - stack_page_virt);
    }
    sp &= !7; // 8-byte align downward
    for i in (0..argc).rev() {
        sp -= 8;
        *(sp as *mut u64) = ptrs[i];
    }
    sp -= 8;
    *(sp as *mut u64) = 0; // argv[argc] = NULL
    sp -= 8;
    *(sp as *mut u64) = argc as u64; // argc
    let new_user_rsp = page_base_virt + (sp - stack_page_virt);

    crate::serial::_print(format_args!(
        "[exec] pid={} image replaced: {} → entry={:#x}, rsp={:#x}, cr3={:#x}, argc={}\n",
        crate::process::PROCESS_TABLE.lock().current_pid,
        path_str,
        entry,
        new_user_rsp,
        new_cr3,
        argc
    ));

    // Free the OLD user address space (its frames are no longer reachable
    // once we switch; kernel pages are shared and untouched).
    {
        let table = crate::process::PROCESS_TABLE.lock();
        let pid = table.current_pid;
        let old_cr3 = table.get(pid).map(|p| p.cr3).unwrap_or(0);
        drop(table);
        if old_cr3 != 0 && old_cr3 != new_cr3 {
            if let Ok(frame) = x86_64::structures::paging::PhysFrame::<x86_64::structures::paging::Size4KiB>::from_start_address(
                x86_64::PhysAddr::new(old_cr3),
            ) {
                crate::memory::page_table::AddressSpace { pml4_frame: frame }.dealloc_user();
            }
        }
    }

    // Update the process record and rebuild its kernel-stack context so
    // that the next switch re-enters Ring3 at the new entry point.
    {
        let mut table = crate::process::PROCESS_TABLE.lock();
        let pid = table.current_pid;
        let name = path_str.rsplit('/').next().unwrap_or(path_str);
        if let Some(proc) = table.get_mut(pid) {
            proc.entry_point = entry;
            proc.user_rsp = new_user_rsp;
            proc.cr3 = new_cr3;
            let mut nb = [0u8; 16];
            let nlen = core::cmp::min(name.len(), 15);
            nb[..nlen].copy_from_slice(&name.as_bytes()[..nlen]);
            proc.name = nb;
            proc.state = crate::process::ProcessState::Ready;
        }
        let slot = table
            .processes
            .iter()
            .position(|p| p.pid == pid)
            .unwrap_or(0);
        drop(table);
        crate::scheduler::init_process_stack(slot, crate::userproc::user_trampoline as *const () as u64);
    }

    // Switch into the new context (never returns).
    crate::scheduler::restart_current();

    // Unreachable in practice.
    0
}

/// `getdents(path, buf, count)` — list a directory into a user buffer.
///
/// Each entry is 72 bytes: [0] type (1=file, 2=dir, 3=char, 4=block),
/// [1..65] name (NUL-terminated), [65..72] zero. Returns bytes written
/// (n * 72), or a negative errno.
unsafe fn sys_getdents(path: *const u8, buf: *mut u8, count: u64) -> i64 {
    let path_str = match read_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if buf.is_null() {
        return Errno::efault.as_i64();
    }
    let vfs = crate::fs::VFS.lock();
    let entries = match vfs.readdir(&path_str) {
        Ok(e) => e,
        Err(_) => return -20, // ENOTDIR
    };
    let mut written: usize = 0;
    for e in entries {
        if written + 72 > count as usize {
            break;
        }
        let ty = match e.file_type {
            crate::fs::FileType::Regular => 1,
            crate::fs::FileType::Directory => 2,
            crate::fs::FileType::CharDevice => 3,
            crate::fs::FileType::BlockDevice => 4,
        };
        *buf.add(written) = ty;
        let name = e.name.as_bytes();
        let n = core::cmp::min(name.len(), 63);
        core::ptr::copy_nonoverlapping(name.as_ptr(), buf.add(written + 1), n);
        *buf.add(written + 1 + n) = 0;
        core::ptr::write_bytes(buf.add(written + 65), 0, 7);
        written += 72;
    }
    written as i64
}

/// `getprocs(buf, count)` — snapshot the process table into a user
/// buffer. Entry layout (40 bytes):
///   [0..8)  pid
///   [8..16) parent pid
///   [16..24) state (1 Ready, 2 Running, 3 Blocked, 4 Zombie)
///   [24..40) name (16 bytes, zero-padded)
/// Returns the number of entries written, or a negative errno.
unsafe fn sys_getprocs(buf: *mut u8, count: u64) -> i64 {
    if buf.is_null() {
        return Errno::efault.as_i64();
    }
    let procs = crate::process::PROCESS_TABLE.lock();
    let mut written: usize = 0;
    for p in procs.processes.iter() {
        if p.pid == 0 {
            continue;
        }
        if written >= count as usize {
            break;
        }
        let base = unsafe { buf.add(written * 40) };
        unsafe {
            core::ptr::write_unaligned(base as *mut u64, p.pid as u64);
            core::ptr::write_unaligned(base.add(8) as *mut u64, p.parent_pid as u64);
            core::ptr::write_unaligned(base.add(16) as *mut u64, p.state as u64);
            core::ptr::copy_nonoverlapping(p.name.as_ptr(), base.add(24), 16);
        }
        written += 1;
    }
    written as i64
}

/// `getpid()` → current process ID.
unsafe fn sys_getpid() -> i64 {
    crate::process::PROCESS_TABLE.lock().current_pid as i64
}

/// `close(fd)` → 0 on success, negative errno on failure.
unsafe fn sys_close(fd: i32) -> i64 {
    if fd < 3 {
        // Don't allow closing stdin/stdout/stderr.
        return Errno::ebadf.as_i64();
    }
    if with_fd_table_mut(|table| table.close(fd as usize)) {
        0
    } else {
        Errno::ebadf.as_i64()
    }
}

/// Read a null-terminated UTF-8 path from user memory. Returns (String, len).
unsafe fn read_path(path: *const u8) -> Result<alloc::string::String, i64> {
    if path.is_null() {
        return Err(Errno::efault.as_i64());
    }
    let mut len = 0usize;
    while *path.add(len) != 0 {
        len += 1;
        if len > 255 {
            return Err(Errno::einval.as_i64());
        }
    }
    let slice = core::slice::from_raw_parts(path, len);
    core::str::from_utf8(slice)
        .map(|s| s.to_owned())
        .map_err(|_| Errno::einval.as_i64())
}

/// `mkdir(path)` → 0 or negative errno.
unsafe fn sys_mkdir(path: *const u8) -> i64 {
    let path_str = match read_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let vfs = crate::fs::VFS.lock();
    let (mount_idx, _) = match vfs.find_mount(&path_str) {
        Some(m) => m,
        None => return Errno::enoent.as_i64(),
    };
    let parent = path_str
        .rsplit_once('/')
        .map(|(p, _)| p.to_owned())
        .unwrap_or_else(|| "/".to_owned());
    let name = path_str.rsplit('/').next().unwrap_or("").to_owned();
    let (_, parent_inode) = match vfs.resolve(&parent) {
        Ok(v) => v,
        Err(_) => return Errno::enoent.as_i64(),
    };
    match vfs.mounts[mount_idx].ops.mkdir(parent_inode, &name) {
        Ok(_) => 0,
        Err(_) => Errno::einval.as_i64(),
    }
}

/// `unlink(path)` → 0 or negative errno.
unsafe fn sys_unlink(path: *const u8) -> i64 {
    let path_str = match read_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let vfs = crate::fs::VFS.lock();
    let (mount_idx, _) = match vfs.find_mount(&path_str) {
        Some(m) => m,
        None => return Errno::enoent.as_i64(),
    };
    let parent = path_str
        .rsplit_once('/')
        .map(|(p, _)| p.to_owned())
        .unwrap_or_else(|| "/".to_owned());
    let name = path_str.rsplit('/').next().unwrap_or("").to_owned();
    let (_, parent_inode) = match vfs.resolve(&parent) {
        Ok(v) => v,
        Err(_) => return Errno::enoent.as_i64(),
    };
    match vfs.mounts[mount_idx].ops.unlink(parent_inode, &name) {
        Ok(_) => 0,
        Err(_) => Errno::enoent.as_i64(),
    }
}

/// POSIX-style stat structure (as returned to user space).
#[repr(C)]
pub struct StatBuf {
    pub st_size: u64,
    pub st_mode: u32,
}

/// `stat(path, buf)` → 0 or negative errno.
unsafe fn sys_stat(path: *const u8, buf: *mut StatBuf) -> i64 {
    let path_str = match read_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if buf.is_null() {
        return Errno::efault.as_i64();
    }
    let vfs = crate::fs::VFS.lock();
    let (mount_idx, inode_id) = match vfs.resolve(&path_str) {
        Ok(v) => v,
        Err(_) => return Errno::enoent.as_i64(),
    };
    let ft = vfs.mounts[mount_idx].ops.file_type(inode_id);
    let size = vfs.mounts[mount_idx].ops.size(inode_id);
    let mode = match ft {
        crate::fs::FileType::Directory => 0o040000, // S_IFDIR
        crate::fs::FileType::Regular => 0o100000,   // S_IFREG
        crate::fs::FileType::CharDevice => 0o020000,
        crate::fs::FileType::BlockDevice => 0o060000,
    };
    (*buf).st_size = size;
    (*buf).st_mode = mode;
    0
}

/// Per-process current working directory (single global for now).
pub static CURRENT_DIR: spin::Mutex<alloc::string::String> =
    spin::Mutex::new(alloc::string::String::new());

/// `chdir(path)` → 0 or negative errno.
unsafe fn sys_chdir(path: *const u8) -> i64 {
    let path_str = match read_path(path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let vfs = crate::fs::VFS.lock();
    match vfs.resolve(&path_str) {
        Ok((m_idx, inode)) => {
            if vfs.mounts[m_idx].ops.file_type(inode) != crate::fs::FileType::Directory {
                return -20; // ENOTDIR
            }
            let mut cwd = CURRENT_DIR.lock();
            *cwd = path_str;
            0
        }
        Err(_) => Errno::enoent.as_i64(),
    }
}

/// `getcwd(buf, size)` → 0 or negative errno.
unsafe fn sys_getcwd(buf: *mut u8, size: u64) -> i64 {
    if buf.is_null() || size == 0 {
        return Errno::efault.as_i64();
    }
    let cwd = CURRENT_DIR.lock();
    let bytes = cwd.as_bytes();
    if bytes.len() + 1 > size as usize {
        return Errno::einval.as_i64();
    }
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
    *buf.add(bytes.len()) = 0;
    0
}

/// `fork()` → 0 in the child, the child's PID in the parent, negative
/// errno on failure.
///
/// Duplicates the calling process:
/// - a fresh user address space with identical page contents (copy of
///   the parent's user pages; copy-on-write is a future optimisation)
/// - a copy of the parent's file descriptor table
/// - a kernel stack that resumes the child right after its `int 0x80`
///   with `rax = 0`, so user code can branch on the fork return value.
unsafe fn sys_fork(ctx: *mut crate::syscall_trampoline::InterruptContext) -> i64 {
    let parent_pid = crate::process::PROCESS_TABLE.lock().current_pid;
    if parent_pid == 0 {
        // The kernel itself cannot fork.
        return Errno::enosys.as_i64();
    }
    if ctx.is_null() {
        return Errno::einval.as_i64();
    }
    let ctx = &*ctx;

    // 1. Clone the parent's user address space.
    let parent_cr3 = {
        let table = crate::process::PROCESS_TABLE.lock();
        match table.get(parent_pid) {
            Some(p) if p.cr3 != 0 => p.cr3,
            _ => return Errno::enosys.as_i64(),
        }
    };
    let parent_as = match crate::memory::page_table::AddressSpace::from_cr3(parent_cr3) {
        Some(a) => a,
        None => return Errno::enosys.as_i64(),
    };
    let child_as = match parent_as.clone_user_space() {
        Ok(a) => a,
        Err(_) => return Errno::enomem.as_i64(),
    };
    let child_cr3 = child_as.pml4_frame.start_address().as_u64();

    // 2. Allocate the child slot and copy parent fields.
    x86_64::instructions::interrupts::disable();
    let (child_pid, child_slot) = {
        let mut table = crate::process::PROCESS_TABLE.lock();
        let (parent_name, parent_entry, parent_fd) = {
            let parent = table.get_mut(parent_pid).unwrap();
            (parent.name, parent.entry_point, parent.fd_table)
        };
        let pid = table.alloc(parent_pid, b"fork");
        if pid == 0 {
            return Errno::enomem.as_i64();
        }
        let slot = table
            .processes
            .iter()
            .position(|p| p.pid == pid)
            .unwrap();
        {
            let proc = table.get_mut(pid).unwrap();
            proc.entry_point = parent_entry;
            proc.user_rsp = ctx.rsp;
            proc.cr3 = child_cr3;
            proc.fd_table = parent_fd;
            proc.set_name(&parent_name);
            proc.state = crate::process::ProcessState::Ready;
        }
        (pid, slot)
    };
    x86_64::instructions::interrupts::enable();

    // 3. Build the child's kernel stack: switch_context returns at
    //    syscall_trampoline_after_call, which pops the GPRs and iretq's
    //    back to user mode right after the int 0x80, rax = 0.
    {
        let mut stacks = crate::scheduler::KERNEL_STACKS.lock();
        let stack = &mut stacks[child_slot];
        let top = stack.stack_top();
        let mut sp = top as *mut u64;

        // CPU-pushed frame (iretq pops RIP, CS, RFLAGS, RSP, SS; memory
        // low→high is RIP, CS, RFLAGS, RSP, SS — write high→low).
        sp = sp.sub(1);
        *sp = ctx.ss;
        sp = sp.sub(1);
        *sp = ctx.rsp;
        sp = sp.sub(1);
        *sp = ctx.rflags;
        sp = sp.sub(1);
        *sp = ctx.cs;
        sp = sp.sub(1);
        *sp = ctx.rip;

        // GPR save area. The trampoline pops r15..rax in that order, so
        // memory low→high is r15..rax. Write high→low (rax first).
        let gprs = [
            ctx.r15, ctx.r14, ctx.r13, ctx.r12, ctx.r11, ctx.r10, ctx.r9,
            ctx.r8, ctx.rbp, ctx.rdi, ctx.rsi, ctx.rdx, ctx.rcx, ctx.rbx,
            0u64, // rax = 0 → child sees fork() == 0
        ];
        for reg in gprs.iter().rev() {
            sp = sp.sub(1);
            *sp = *reg;
        }

        // Callee-saved block + return address for switch_context
        // (pop order r15, r14, r13, r12, rbx, rbp, then ret).
        let after_call =
            crate::syscall_trampoline::syscall_trampoline_after_call as *const () as u64;
        sp = sp.sub(1);
        *sp = after_call; // ret target
        sp = sp.sub(1);
        *sp = 0; // rbp
        sp = sp.sub(1);
        *sp = 0; // rbx
        sp = sp.sub(1);
        *sp = 0; // r12
        sp = sp.sub(1);
        *sp = 0; // r13
        sp = sp.sub(1);
        *sp = 0; // r14
        sp = sp.sub(1);
        *sp = 0; // r15 (lowest address → saved_rsp)

        stack.saved_rsp = sp as u64;
        stack.initialised = true;
    }
    // The parent returns the child's PID.
    child_pid as i64
}

/// `waitpid(pid, status_ptr)` → reaped child PID on success.
///
/// pid = -1 (u64::MAX) waits for any child. While the child is still
/// running the calling process yields via `hlt`; the timer interrupt
/// keeps the scheduler running other processes, and we are switched back
/// when our slice returns.
unsafe fn sys_waitpid(pid: u64, status_ptr: u64) -> i64 {
    loop {
        let result = {
            let table = crate::process::PROCESS_TABLE.lock();
            let current = table.current_pid;
            let mut result: Option<(u32, i32, bool)> = None;
            for p in table.processes.iter() {
                if p.pid != 0 && p.parent_pid == current {
                    if pid == u64::MAX || p.pid as u64 == pid {
                        result = Some((
                            p.pid,
                            p.exit_code,
                            p.state == crate::process::ProcessState::Zombie,
                        ));
                        break;
                    }
                }
            }
            result
        };
        match result {
            Some((child_pid, code, true)) => {
                if status_ptr != 0 {
                    *(status_ptr as *mut i32) = code;
                }
                crate::process::PROCESS_TABLE.lock().free(child_pid);
                return child_pid as i64;
            }
            Some(_) => {
                // Child still alive: yield to the scheduler so other
                // processes can run. A bare hlt would NOT reschedule —
                // the scheduler only switches when the slice expires or
                // the current process dies.
                crate::scheduler::yield_now();
            }
            None => {
                // No such child.
                return -10; // ECHILD
            }
        }
    }
}

/// `getppid()` → parent process ID (0 = kernel).
unsafe fn sys_getppid() -> i64 {
    let table = crate::process::PROCESS_TABLE.lock();
    let current = table.current_pid;
    if current == 0 {
        return 0;
    }
    table.get(current).map_or(0, |p| p.parent_pid as i64)
}


