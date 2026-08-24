//! Input event structures shared between kernel and user-mode WM.
//!
//! The `InputEvent` struct is `#[repr(C)]` so the user-mode window
//! manager can read it directly from the `poll_input` syscall.
//!
//! The kernel converts raw PS/2 scancodes and mouse packets into
//! these structured events. No window-management logic here —
//! the kernel only reports hardware events.

/// Event type: keyboard.
pub const EVENT_KEYBOARD: u32 = 0;
/// Event type: mouse.
pub const EVENT_MOUSE: u32 = 1;

/// Structured input event returned by `sys_poll_input`.
///
/// For keyboard events: `event_type = EVENT_KEYBOARD`,
///   `keycode`, `key_char`, `key_pressed` are valid.
///
/// For mouse events: `event_type = EVENT_MOUSE`,
///   `mouse_buttons`, `mouse_dx`, `mouse_dy` are valid.
///
/// Layout (20 bytes total):
/// ```text
/// offset 0:  u32 event_type
/// offset 4:  u32 keycode
/// offset 8:  u32 key_char
/// offset 12: u8  key_pressed
/// offset 13: u8  mouse_buttons
/// offset 14: i16 mouse_dx
/// offset 16: i16 mouse_dy
/// ```
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct InputEvent {
    /// `EVENT_KEYBOARD` or `EVENT_MOUSE`.
    pub event_type: u32,
    /// Keyboard: PS/2 Set-1 make code (release bit stripped).
    pub keycode: u32,
    /// Keyboard: Unicode codepoint (0 if non-printable).
    pub key_char: u32,
    /// Keyboard: 1 = key pressed, 0 = key released.
    pub key_pressed: u8,
    /// Mouse: bit 0 = left, bit 1 = right, bit 2 = middle.
    pub mouse_buttons: u8,
    /// Mouse: X delta since last poll (positive = right).
    pub mouse_dx: i16,
    /// Mouse: Y delta since last poll (positive = up).
    pub mouse_dy: i16,
}

impl Default for InputEvent {
    fn default() -> Self {
        Self {
            event_type: EVENT_KEYBOARD,
            keycode: 0,
            key_char: 0,
            key_pressed: 0,
            mouse_buttons: 0,
            mouse_dx: 0,
            mouse_dy: 0,
        }
    }
}

/// Poll for the next input event. Fills `event` and returns 1 if an
/// event is available, returns 0 if no events pending.
///
/// Checks keyboard scancode buffer first, then mouse movement/buttons.
pub fn poll_next(event: &mut InputEvent) -> i64 {
    // 1. Try keyboard.
    if let Some(scancode) = crate::interrupts::SCANCODE_BUFFER.lock().pop() {
        let is_release = (scancode & 0x80) != 0;
        let code = scancode & 0x7f;
        // Skip E0/E1 extended key prefixes — the next byte is the real code.
        if scancode == 0xe0 || scancode == 0xe1 {
            return poll_next(event); // skip prefix, recurse
        }
        let keychar = crate::scancode_to_ascii(code)
            .map(|c| c as u32)
            .unwrap_or(0);
        *event = InputEvent {
            event_type: EVENT_KEYBOARD,
            keycode: code as u32,
            key_char: keychar,
            key_pressed: if is_release { 0 } else { 1 },
            mouse_buttons: 0,
            mouse_dx: 0,
            mouse_dy: 0,
        };
        return 1;
    }

    // 2. Try mouse.
    let (dx, dy, buttons) = crate::interrupts::mouse::drain_mouse();
    if dx != 0 || dy != 0 || buttons != 0 {
        *event = InputEvent {
            event_type: EVENT_MOUSE,
            keycode: 0,
            key_char: 0,
            key_pressed: 0,
            mouse_buttons: buttons,
            mouse_dx: dx,
            mouse_dy: dy,
        };
        return 1;
    }

    // No events.
    0
}
