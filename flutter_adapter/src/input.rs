//! Input adapter — converts kernel PS/2 scancodes to Flutter events.
//!
//! ## Flow
//!
//! 1. Kernel PS/2 keyboard IRQ → scancode buffer (Ring0)
//! 2. User-mode `read(0, buf, N)` → get raw scancodes (syscall)
//! 3. This module converts scancodes → `FlutterKeyEvent` / `FlutterPointerEvent`
//! 4. Events pushed to Flutter Engine via embedder API
//!
//! ## Flutter event types
//!
//! Flutter Engine expects:
//! - `FlutterPointerEvent` (mouse/touch: phase, x, y, buttons)
//! - `FlutterKeyEvent` (keyboard: keycode, keychar, phase)
//!
//! Our PS/2 keyboard has no mouse, so we synthesise pointer events
//! from arrow keys for basic navigation testing.
//!
//! [MANUAL] The real Flutter event structs come from `flutter_embedder.h`.
//! We re-define minimal compatible versions here.

use crate::syscalls;

/// Pointer event phase (matches Flutter's FlutterPointerPhase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PointerPhase {
    Cancel = 0,
    Add = 1,
    Hover = 2,
    Down = 3,
    Up = 4,
    Remove = 5,
}

/// Key event phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum KeyPhase {
    Down = 0,
    Up = 1,
    Repeat = 2,
}

/// Flutter-compatible pointer event.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FlutterPointerEvent {
    pub phase: PointerPhase,
    pub x: f64,
    pub y: f64,
    pub timestamp: u64, // microseconds since epoch
}

/// Flutter-compatible key event.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FlutterKeyEvent {
    pub phase: KeyPhase,
    pub keycode: u32,   // PS/2 scancode or USB HID code
    pub keychar: u32,   // Unicode codepoint (0 if non-printable)
    pub timestamp: u64,
}

/// Unified input event.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    Pointer(FlutterPointerEvent),
    Key(FlutterKeyEvent),
}

/// Maximum events returned per poll.
pub const MAX_EVENTS: usize = 32;

/// A simple event list with a fixed capacity (no alloc needed).
pub struct EventList {
    events: [InputEvent; MAX_EVENTS],
    len: usize,
}

impl EventList {
    pub const fn new() -> Self {
        Self {
            events: [InputEvent::Key(FlutterKeyEvent {
                phase: KeyPhase::Down,
                keycode: 0,
                keychar: 0,
                timestamp: 0,
            }); MAX_EVENTS],
            len: 0,
        }
    }

    pub fn push(&mut self, event: InputEvent) {
        if self.len < MAX_EVENTS {
            self.events[self.len] = event;
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &InputEvent> {
        self.events[..self.len].iter()
    }
}

/// Input adapter state.
pub struct InputAdapter {
    /// Cursor position for synthesised pointer events.
    cursor_x: f64,
    cursor_y: f64,
    /// Current scancode buffer from kernel.
    scancode_buf: [u8; 64],
}

impl InputAdapter {
    pub const fn new() -> Self {
        Self {
            cursor_x: 0.0,
            cursor_y: 0.0,
            scancode_buf: [0; 64],
        }
    }
}

static mut ADAPTER: InputAdapter = InputAdapter::new();

/// Initialise the input adapter.
pub fn init() -> Result<(), i32> {
    // Verify we can read from stdin (fd=0).
    let mut test = [0u8; 1];
    let ret = syscalls::read(0, &mut test);
    if ret < 0 && ret != syscalls::ENOSYS {
        // ENOSYS is expected in skeleton mode — that's OK.
        // Other errors mean the syscall interface is broken.
        return Err(ret as i32);
    }
    Ok(())
}

/// Poll for input events from the kernel.
///
/// Reads raw scancodes via `read(0, ...)` syscall, converts to
/// Flutter-compatible events. Returns a slice of events.
///
/// [MANUAL] When Ring3 is ready, `syscalls::read` will actually
/// invoke `int 0x80`. For now, it returns ENOSYS and we return
/// an empty list.
pub fn poll() -> EventList {
    let mut events = EventList::new();

    // SAFETY: single-threaded user-mode skeleton.
    let adapter = unsafe { &mut ADAPTER };

    // Read raw scancodes from kernel stdin.
    let n = syscalls::read(0, &mut adapter.scancode_buf);
    if n <= 0 {
        return events;
    }

    let count = n as usize;
    for i in 0..count {
        let sc = adapter.scancode_buf[i];
        // Bit 7 set = key release; clear = key press.
        let is_release = (sc & 0x80) != 0;
        let scancode = sc & 0x7f;

        // Skip E0/E1 prefixes (extended keys).
        if sc == 0xe0 || sc == 0xe1 {
            continue;
        }

        // Convert to key event.
        let phase = if is_release {
            KeyPhase::Up
        } else {
            KeyPhase::Down
        };

        let keychar = scancode_to_char(scancode).unwrap_or(0);

        let timestamp = get_timestamp();

        events.push(InputEvent::Key(FlutterKeyEvent {
            phase,
            keycode: scancode as u32,
            keychar,
            timestamp,
        }));

        // Synthesise pointer events from arrow keys.
        if !is_release {
            match scancode {
                0x4b => {
                    // Left arrow
                    adapter.cursor_x = (adapter.cursor_x - 5.0).max(0.0);
                    events.push(InputEvent::Pointer(FlutterPointerEvent {
                        phase: PointerPhase::Hover,
                        x: adapter.cursor_x,
                        y: adapter.cursor_y,
                        timestamp,
                    }));
                }
                0x4d => {
                    // Right arrow
                    adapter.cursor_x += 5.0;
                    events.push(InputEvent::Pointer(FlutterPointerEvent {
                        phase: PointerPhase::Hover,
                        x: adapter.cursor_x,
                        y: adapter.cursor_y,
                        timestamp,
                    }));
                }
                0x48 => {
                    // Up arrow
                    adapter.cursor_y = (adapter.cursor_y - 5.0).max(0.0);
                    events.push(InputEvent::Pointer(FlutterPointerEvent {
                        phase: PointerPhase::Hover,
                        x: adapter.cursor_x,
                        y: adapter.cursor_y,
                        timestamp,
                    }));
                }
                0x50 => {
                    // Down arrow
                    adapter.cursor_y += 5.0;
                    events.push(InputEvent::Pointer(FlutterPointerEvent {
                        phase: PointerPhase::Hover,
                        x: adapter.cursor_x,
                        y: adapter.cursor_y,
                        timestamp,
                    }));
                }
                0x1c => {
                    // Enter → pointer down
                    events.push(InputEvent::Pointer(FlutterPointerEvent {
                        phase: PointerPhase::Down,
                        x: adapter.cursor_x,
                        y: adapter.cursor_y,
                        timestamp,
                    }));
                }
                _ => {}
            }
        }
    }

    events
}

/// Map PS/2 Set-1 scancode to Unicode codepoint (printable keys only).
fn scancode_to_char(sc: u8) -> Option<u32> {
    Some(match sc {
        0x02 => b'1' as u32, 0x03 => b'2' as u32, 0x04 => b'3' as u32,
        0x05 => b'4' as u32, 0x06 => b'5' as u32, 0x07 => b'6' as u32,
        0x08 => b'7' as u32, 0x09 => b'8' as u32, 0x0a => b'9' as u32,
        0x0b => b'0' as u32, 0x0c => b'-' as u32, 0x0d => b'=' as u32,
        0x10 => b'q' as u32, 0x11 => b'w' as u32, 0x12 => b'e' as u32,
        0x13 => b'r' as u32, 0x14 => b't' as u32, 0x15 => b'y' as u32,
        0x16 => b'u' as u32, 0x17 => b'i' as u32, 0x18 => b'o' as u32,
        0x19 => b'p' as u32, 0x1a => b'[' as u32, 0x1b => b']' as u32,
        0x1e => b'a' as u32, 0x1f => b's' as u32, 0x20 => b'd' as u32,
        0x21 => b'f' as u32, 0x22 => b'g' as u32, 0x23 => b'h' as u32,
        0x24 => b'j' as u32, 0x25 => b'k' as u32, 0x26 => b'l' as u32,
        0x27 => b';' as u32, 0x28 => b'\'' as u32, 0x29 => b'`' as u32,
        0x2b => b'\\' as u32,
        0x2c => b'z' as u32, 0x2d => b'x' as u32, 0x2e => b'c' as u32,
        0x2f => b'v' as u32, 0x30 => b'b' as u32, 0x31 => b'n' as u32,
        0x32 => b'm' as u32, 0x33 => b',' as u32, 0x34 => b'.' as u32,
        0x35 => b'/' as u32, 0x39 => b' ' as u32, 0x1c => b'\n' as u32,
        _ => return None,
    })
}

/// Get a microsecond timestamp via `clock_gettime(MONOTONIC)`.
fn get_timestamp() -> u64 {
    let mut ts = [0i64; 2];
    let ret = syscalls::clock_gettime(1, &mut ts as *mut i64 as u64);
    if ret < 0 {
        return 0;
    }
    // tv_sec * 1_000_000 + tv_nsec / 1000
    ts[0] as u64 * 1_000_000 + (ts[1] as u64 / 1000)
}
