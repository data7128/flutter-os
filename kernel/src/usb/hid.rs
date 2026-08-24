//! USB HID keyboard and mouse driver.
//!
//! Supports HID Boot Protocol devices (keyboard and mouse) via
//! UHCI interrupt transfers. The boot protocol is a simplified
//! report format defined in the USB HID specification:
//!
//! - Keyboard: 8-byte report (modifier, reserved, 6 keycodes)
//! - Mouse: 4-byte report (buttons, dx, dy, wheel)
//!
//! ## Limitations
//! - Only boot protocol (no report descriptor parsing)
//! - Only keyboard + mouse (no gamepads, touchscreens, etc.)
//! - No USB hubs (direct connection only)
//!
//! [MANUAL] The full HID driver requires:
//! - SET_IDLE control transfer to suppress repeating reports
//! - SET_PROTOCOL(BOOT) to enable boot protocol
//! - Periodic interrupt transfers (polling every 8ms for keyboard,
//!   10ms for mouse)
//! - UHCI TD scheduling for interrupt endpoint

use crate::interrupts::SCANCODE_BUFFER;
use crate::interrupts::mouse::MOUSE_STATE;

/// USB HID device types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidType {
    Keyboard,
    Mouse,
    Unknown,
}

/// HID boot keyboard report (8 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct KeyboardReport {
    /// Modifier keys: bit 0=LCtrl, 1=LShift, 2=LAlt, 3=LGUI,
    /// 4=RCtrl, 5=RShift, 6=RAlt, 7=RGUI.
    pub modifiers: u8,
    /// Reserved (always 0 in boot protocol).
    pub _reserved: u8,
    /// Up to 6 simultaneously pressed keycodes (0 = no key).
    pub keycodes: [u8; 6],
}

/// HID boot mouse report (4 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct MouseReport {
    /// Buttons: bit 0=left, 1=right, 2=middle.
    pub buttons: u8,
    /// X movement delta (int8, range -127..127).
    pub dx: i8,
    /// Y movement delta (int8, range -127..127, positive = down).
    pub dy: i8,
    /// Wheel delta (int8, usually -1, 0, or 1).
    pub wheel: i8,
}

/// Convert USB HID keycode (Usage Page 0x07) to PS/2 Set-1 scancode.
///
/// This lets the existing PS/2 input subsystem process USB keyboard
/// events transparently — the kernel sees the same scancode format
/// from both PS/2 and USB keyboards.
fn hid_to_scancode(hid_keycode: u8) -> Option<u8> {
    // USB HID Usage Table → PS/2 Set-1 scancode mapping (subset).
    Some(match hid_keycode {
        0x04 => 0x1E, // a
        0x05 => 0x30, // b
        0x06 => 0x2E, // c
        0x07 => 0x20, // d
        0x08 => 0x12, // e
        0x09 => 0x21, // f
        0x0A => 0x22, // g
        0x0B => 0x23, // h
        0x0C => 0x17, // i
        0x0D => 0x24, // j
        0x0E => 0x25, // k
        0x0F => 0x26, // l
        0x10 => 0x32, // m
        0x11 => 0x31, // n
        0x12 => 0x18, // o
        0x13 => 0x19, // p
        0x14 => 0x10, // q
        0x15 => 0x13, // r
        0x16 => 0x1F, // s
        0x17 => 0x14, // t
        0x18 => 0x16, // u
        0x19 => 0x2F, // v
        0x1A => 0x11, // w
        0x1B => 0x2D, // x
        0x1C => 0x15, // y
        0x1D => 0x2C, // z
        0x1E => 0x02, // 1
        0x1F => 0x03, // 2
        0x20 => 0x04, // 3
        0x21 => 0x05, // 4
        0x22 => 0x06, // 5
        0x23 => 0x07, // 6
        0x24 => 0x08, // 7
        0x25 => 0x09, // 8
        0x26 => 0x0A, // 9
        0x27 => 0x0B, // 0
        0x28 => 0x1C, // Enter
        0x29 => 0x01, // Esc
        0x2A => 0x0E, // Backspace
        0x2B => 0x0F, // Tab
        0x2C => 0x39, // Space
        0x4F => 0x50, // Right arrow (→) (hid: 0x4F, scancode: 0x50 = numpad)
        0x50 => 0x4D, // Left arrow (←)
        0x51 => 0x4B, // Down arrow (↓)
        0x52 => 0x48, // Up arrow (↑)
        _ => return None,
    })
}

/// Process a USB keyboard report.
///
/// Converts HID keycodes to PS/2 scancodes and pushes them into the
/// same SCANCODE_BUFFER used by the PS/2 keyboard interrupt handler.
/// This makes the input subsystem transparent — upper layers see
/// a unified scancode stream regardless of input source.
pub fn process_keyboard_report(report: &KeyboardReport) {
    // For each keycode in the report, convert to scancode and push.
    for &kc in &report.keycodes {
        if kc == 0 {
            continue; // Empty slot.
        }
        if let Some(sc) = hid_to_scancode(kc) {
            // Push as "make" (key press). Release detection would
            // require comparing with previous report.
            SCANCODE_BUFFER.lock().push(sc);
        }
    }
}

/// Process a USB mouse report.
///
/// Converts the HID mouse delta to the same format as the PS/2
/// mouse state, so the existing `poll_input` syscall works for both.
pub fn process_mouse_report(report: &MouseReport) {
    let mut state = MOUSE_STATE.lock();

    // USB mouse Y is positive=down, PS/2 after our inversion is
    // positive=up. We need to invert the USB Y delta.
    let dx = report.dx as i16;
    let dy = -(report.dy as i16); // Invert Y to match PS/2 convention.

    state.dx = state.dx.saturating_add(dx);
    state.dy = state.dy.saturating_add(dy);
    state.buttons = report.buttons & 0x07;
}

/// Initialize HID drivers.
///
/// [MANUAL] The real init requires:
/// - USB device enumeration (SET_ADDRESS, SET_CONFIGURATION)
/// - SET_IDLE and SET_PROTOCOL(BOOT) control transfers
/// - Setting up periodic interrupt transfers via UHCI
/// - IRQ handling for UHCI completion interrupts
pub fn init() {
    crate::serial::_print(format_args!(
        "[usb-hid] HID driver skeleton initialized\n"
    ));
    crate::serial::_print(format_args!(
        "[usb-hid] [MANUAL] device enumeration requires UHCI transfers\n"
    ));
}
