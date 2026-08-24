//! WM syscall wrappers — thin stubs around `int 0x80`.
//!
//! These mirror the kernel's `SyscallNum` enum. In skeleton mode
//! they return ENOSYS; when Ring3 is implemented, they will use
//! inline `asm!("int 0x80", ...)`.

/// Syscall numbers — must match kernel `syscalls::SyscallNum`.
pub const SYS_WRITE: u64 = 3;
pub const SYS_MMAP: u64 = 4;
pub const SYS_NANOSLEEP: u64 = 5;
pub const SYS_GET_FB_INFO: u64 = 7;
pub const SYS_FB_COMMIT: u64 = 8;
pub const SYS_POLL_INPUT: u64 = 9;

/// Errno values — must match kernel `Errno`.
pub const ENOSYS: i64 = -38;
pub const EBADF: i64 = -9;
pub const EINVAL: i64 = -22;
pub const ENOMEM: i64 = -12;
pub const EFAULT: i64 = -14;

/// Framebuffer info returned by `SYS_GET_FB_INFO`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bytes_per_pixel: u32,
    pub format: u32,
    pub len: u64,
}

impl Default for FramebufferInfo {
    fn default() -> Self {
        Self {
            address: 0,
            width: 0,
            height: 0,
            stride: 0,
            bytes_per_pixel: 0,
            format: 0,
            len: 0,
        }
    }
}

/// Input event type constants (match kernel `syscalls::input`).
pub const EVENT_KEYBOARD: u32 = 0;
pub const EVENT_MOUSE: u32 = 1;

/// Structured input event (must match kernel layout).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct InputEvent {
    pub event_type: u32,
    pub keycode: u32,
    pub key_char: u32,
    pub key_pressed: u8,
    pub mouse_buttons: u8,
    pub mouse_dx: i16,
    pub mouse_dy: i16,
}

impl Default for InputEvent {
    fn default() -> Self {
        Self {
            event_type: 0,
            keycode: 0,
            key_char: 0,
            key_pressed: 0,
            mouse_buttons: 0,
            mouse_dx: 0,
            mouse_dy: 0,
        }
    }
}

/// `write(fd, buf)` — write to serial/VGA.
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    let _ = (fd, buf);
    ENOSYS
}

/// `mmap(addr, len, prot)` → virtual address or ENOSYS.
pub fn mmap(addr: u64, len: u64, prot: u32) -> i64 {
    let _ = (addr, len, prot);
    ENOSYS
}

/// `nanosleep(ticks)` — sleep for N PIT ticks.
pub fn nanosleep(ticks: u64) -> i64 {
    let _ = ticks;
    ENOSYS
}

/// `get_framebuffer_info(&mut info)` → 0 on success.
pub fn get_framebuffer_info(info: &mut FramebufferInfo) -> i64 {
    let _ = info;
    ENOSYS
}

/// `fb_commit(buf, x, y, w, h)` → 0 on success.
///
/// Blits a rendered buffer to the physical framebuffer.
pub fn fb_commit(buf: &[u8], x: u32, y: u32, w: u32, h: u32) -> i64 {
    let _ = (buf, x, y, w, h);
    ENOSYS
}

/// `poll_input(&mut event)` → 1 if event available, 0 if none.
pub fn poll_input(event: &mut InputEvent) -> i64 {
    let _ = event;
    0 // No events in skeleton mode.
}
