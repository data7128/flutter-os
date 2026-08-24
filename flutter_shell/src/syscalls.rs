//! Flutter Shell syscall wrappers — mirrors kernel syscalls.
//!
//! [MANUAL] All wrappers return ENOSYS until Ring3 is implemented.
//! When Ring3 is ready, replace with `asm!("int 0x80", ...)`.

pub const ENOSYS: i64 = -38;

/// Framebuffer info (must match kernel layout).
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
            address: 0, width: 0, height: 0, stride: 0,
            bytes_per_pixel: 0, format: 0, len: 0,
        }
    }
}

/// Input event (must match kernel layout).
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
            event_type: 0, keycode: 0, key_char: 0, key_pressed: 0,
            mouse_buttons: 0, mouse_dx: 0, mouse_dy: 0,
        }
    }
}

pub const EVENT_KEYBOARD: u32 = 0;
pub const EVENT_MOUSE: u32 = 1;

pub fn get_framebuffer_info(_info: &mut FramebufferInfo) -> i64 { ENOSYS }
pub fn fb_commit(_buf: &[u8], _x: u32, _y: u32, _w: u32, _h: u32) -> i64 { ENOSYS }
pub fn poll_input(_event: &mut InputEvent) -> i64 { 0 }
pub fn nanosleep(_ticks: u64) -> i64 { ENOSYS }
