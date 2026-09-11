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

pub use fd::FdTable;
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
            Some(SyscallNum::Exec) => sys_exec(arg0 as *const u8),
            Some(SyscallNum::Getpid) => sys_getpid(),
            Some(SyscallNum::Close) => sys_close(arg0 as i32),
            Some(SyscallNum::Mkdir) => sys_mkdir(arg0 as *const u8),
            Some(SyscallNum::Unlink) => sys_unlink(arg0 as *const u8),
            Some(SyscallNum::Stat) => sys_stat(arg0 as *const u8, arg1 as *mut StatBuf),
            Some(SyscallNum::Chdir) => sys_chdir(arg0 as *const u8),
            Some(SyscallNum::Getcwd) => sys_getcwd(arg0 as *mut u8, arg1),
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
    let vfs = crate::fs::VFS.lock();
    let file = match vfs.open(path_str, create) {
        Ok(f) => f,
        Err(e) => {
            crate::serial::_print(format_args!("[syscall] open(\"{}\") failed: {}\n", path_str, e));
            return Errno::enoent.as_i64();
        }
    };

    let mut table = FD_TABLE.lock();
    let fd = table.alloc(crate::syscalls::fd::FdKind::VfsFile {
        mount_idx: file.mount_idx,
        inode_id: file.inode_id,
        offset: 0,
    });
    match fd {
        Some(f) => f as i64,
        None => Errno::enomem.as_i64(),
    }
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
            let mut table = FD_TABLE.lock();
            match table.get(fd as usize) {
                Some(entry) => {
                    if let crate::syscalls::fd::FdKind::VfsFile { mount_idx, inode_id, offset } = entry.kind {
                        // Read from VFS file.
                        let vfs = crate::fs::VFS.lock();
                        let mut vfs_file = crate::fs::VfsFile {
                            inode_id, mount_idx, offset, file_type: crate::fs::FileType::Regular,
                        };
                        let n = vfs.read(&mut vfs_file, core::slice::from_raw_parts_mut(buf, count as usize));
                        // Update offset in FD table.
                        table.update_vfs_offset(fd as usize, vfs_file.offset);
                        match n {
                            Ok(bytes) => bytes as i64,
                            Err(_) => Errno::einval.as_i64(),
                        }
                    } else {
                        // Legacy File kind — not yet implemented.
                        Errno::enosys.as_i64()
                    }
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
            // VFS file write.
            let mut table = FD_TABLE.lock();
            match table.get(fd as usize) {
                Some(entry) => {
                    if let crate::syscalls::fd::FdKind::VfsFile { mount_idx, inode_id, offset } = entry.kind {
                        let vfs = crate::fs::VFS.lock();
                        let mut vfs_file = crate::fs::VfsFile {
                            inode_id, mount_idx, offset, file_type: crate::fs::FileType::Regular,
                        };
                        let n = vfs.write(&mut vfs_file, slice);
                        table.update_vfs_offset(fd as usize, vfs_file.offset);
                        match n {
                            Ok(bytes) => bytes as i64,
                            Err(_) => Errno::einval.as_i64(),
                        }
                    } else {
                        Errno::ebadf.as_i64()
                    }
                }
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
/// Releases all process resources:
/// - File descriptors (non-std FDs closed)
/// - Pending signals cleared
/// - Process state set to Zombie
///
/// [MANUAL] In Ring3, this performs an `iretq` back to the kernel
/// scheduler. For now, it just marks the process and halts.
unsafe fn sys_exit(status: i32) -> i64 {
    let current = crate::process::PROCESS_TABLE.lock().current_pid;
    crate::serial::_print(format_args!(
        "[syscall] exit({}) from pid={}\n", status, current
    ));

    // Release resources and mark as zombie.
    crate::process::PROCESS_TABLE.lock().mark_exit(current, status);

    // [MANUAL] In Ring3: switch to next runnable process here.
    // For now, halt — kernel has no scheduler yet.
    crate::hlt_loop();
}

/// `exec(path)` → 0 on success, negative errno on failure.
///
/// Loads an ELF executable and prepares a new process.
unsafe fn sys_exec(path: *const u8) -> i64 {
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

    // Delegate to the user process loader: read ELF from VFS and spawn.
    match crate::userproc::spawn_user_process_from_path(path_str) {
        Ok(pid) => {
            crate::serial::_print(format_args!("[exec] spawned pid={} from \"{}\"\n", pid, path_str));
            pid as i64
        }
        Err(e) => {
            crate::serial::_print(format_args!("[exec] failed to load \"{}\": {}\n", path_str, e));
            Errno::enoent.as_i64()
        }
    }
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
    let mut table = FD_TABLE.lock();
    if table.close(fd as usize) {
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


