//! The kernel-side shell host.
//!
//! On bare metal there is no Flutter engine yet, so the shell host renders
//! a minimal desktop directly onto the linear framebuffer (background
//! gradient, taskbar, a window frame) and runs an input loop that echoes
//! PS/2 keystrokes as colored blocks. This proves the framebuffer + input
//! pipeline end to end.
//!
//! The Flutter desktop shell (see `../shell`) is the real UI: in a full
//! build its engine embedder attaches to this same framebuffer and renders
//! the Dart UI on top of it.

use crate::graphics::{self, Color};
use crate::interrupts::SCANCODE_BUFFER;
use crate::println;

/// Entry point called from `kernel_main`. Never returns.
pub fn launch() -> ! {
    draw_desktop();
    println!("[shell] AeroOS desktop rendered; PS/2 keyboard is live");

    let mut cx: usize = 20;
    let mut cy: usize = 72;
    let mut hue: u8 = 0;

    loop {
        // Halt until the next interrupt (timer / keyboard) wakes us.
        x86_64::instructions::hlt();

        // Drain everything the keyboard IRQ pushed since the last wake.
        while let Some(scancode) = SCANCODE_BUFFER.lock().pop() {
            // Ignore key-release codes (high bit set) and E0/E1 prefixes.
            if scancode >= 0x80 || scancode == 0xe0 || scancode == 0xe1 {
                continue;
            }

            if let Some(ch) = scancode_to_ascii(scancode) {
                println!("[key] '{}'", ch as char);
            } else {
                println!("[key] scan=0x{:02x}", scancode);
            }

            // Echo the keystroke as a colored block on the framebuffer.
            let color = palette(hue);
            hue = hue.wrapping_add(1);
            graphics::fill_rect(cx, cy, 14, 20, color);

            cx += 16;
            let w = graphics::width();
            if cx + 14 > w.saturating_sub(8) {
                cx = 20;
                cy = cy.saturating_add(24);
                if cy + 20 > graphics::height().saturating_sub(40) {
                    cy = 72;
                }
            }
        }
    }
}

/// Paint the initial desktop: gradient background, taskbar, one window.
fn draw_desktop() {
    let w = graphics::width();
    let h = graphics::height();
    if w == 0 || h == 0 {
        println!("[shell] no framebuffer; skipping desktop draw");
        return;
    }

    // Vertical gradient background (deep blue -> near black).
    for y in 0..h {
        let t = (y as u32 * 255 / h.max(1) as u32) as u8;
        graphics::fill_rect(
            0,
            y,
            w,
            1,
            Color {
                r: 0x0bu8.wrapping_add(t / 8),
                g: 0x12u8.wrapping_add(t / 6),
                b: 0x2au8.wrapping_add(t / 4),
            },
        );
    }

    // Top taskbar.
    graphics::fill_rect(0, 0, w, 30, Color::AERO);
    // AeroOS "logo" mark.
    graphics::fill_rect(10, 7, 16, 16, Color::WHITE);

    // A centered window with a title bar + body.
    let wx = w / 4;
    let wy = 64;
    let ww = w / 2;
    let wh = h.saturating_sub(96);
    graphics::fill_rect(wx, wy, ww, 28, Color { r: 0x1e, g: 0x29, b: 0x3b });
    graphics::fill_rect(wx, wy + 28, ww, wh.saturating_sub(28), Color { r: 0x0f, g: 0x17, b: 0x24 });

    // A thin accent line under the taskbar.
    graphics::fill_rect(0, 30, w, 2, Color { r: 0x1d, g: 0x4e, b: 0x89 });
}

/// Map a PS/2 set-1 make code to lowercase ASCII (printable keys only).
fn scancode_to_ascii(sc: u8) -> Option<u8> {
    Some(match sc {
        0x02 => b'1', 0x03 => b'2', 0x04 => b'3', 0x05 => b'4', 0x06 => b'5',
        0x07 => b'6', 0x08 => b'7', 0x09 => b'8', 0x0a => b'9', 0x0b => b'0',
        0x0c => b'-', 0x0d => b'=', 0x0e => 0x08, /* backspace */ 0x0f => b'\t',
        0x10 => b'q', 0x11 => b'w', 0x12 => b'e', 0x13 => b'r', 0x14 => b't',
        0x15 => b'y', 0x16 => b'u', 0x17 => b'i', 0x18 => b'o', 0x19 => b'p',
        0x1a => b'[', 0x1b => b']', 0x1c => b'\n',
        0x1e => b'a', 0x1f => b's', 0x20 => b'd', 0x21 => b'f', 0x22 => b'g',
        0x23 => b'h', 0x24 => b'j', 0x25 => b'k', 0x26 => b'l',
        0x27 => b';', 0x28 => b'\'', 0x29 => b'`', 0x2b => b'\\',
        0x2c => b'z', 0x2d => b'x', 0x2e => b'c', 0x2f => b'v', 0x30 => b'b',
        0x31 => b'n', 0x32 => b'm', 0x33 => b',', 0x34 => b'.', 0x35 => b'/',
        0x39 => b' ',
        _ => return None,
    })
}

/// Cycle through a small palette to color keystroke echoes.
fn palette(h: u8) -> Color {
    match h % 6 {
        0 => Color::AERO,
        1 => Color::GREEN,
        2 => Color::AMBER,
        3 => Color::PINK,
        4 => Color::VIOLET,
        _ => Color::WHITE,
    }
}
