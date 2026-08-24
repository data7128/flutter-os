//! Kernel-side framebuffer shell host.
//!
//! Renders a minimal desktop directly onto the linear framebuffer:
//! gradient background, taskbar, and a window frame. Runs an input loop
//! that echoes PS/2 keystrokes as colored blocks on the framebuffer and
//! text on the serial/VGA console.
//!
//! ## Future GUI integration
//!
//! This module is the **placeholder** for the future Flutter desktop shell.
//! In a full build, the Flutter engine embedder would replace `launch()`
//! and render the Dart UI onto the same framebuffer. The graphics
//! abstraction layer (`graphics` module) remains unchanged.

use crate::graphics::{self, Color};
use crate::interrupts::SCANCODE_BUFFER;
use crate::scancode_to_ascii;
use crate::println;

/// Entry point called from `kernel_main`. Never returns.
///
/// Draws the desktop, then enters a keyboard-echo loop. Each keystroke
/// produces a colored block on the framebuffer and a text log on serial.
pub fn launch() -> ! {
    draw_desktop();
    println!("[shell] framebuffer desktop rendered");

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
///
/// ← FUTURE: The Flutter desktop shell would render a full taskbar with
///   app launcher, clock, system tray, etc. via Dart widgets.
fn draw_desktop() {
    let w = graphics::width();
    let h = graphics::height();
    if w == 0 || h == 0 {
        println!("[shell] no framebuffer; skipping desktop draw");
        return;
    }

    // Vertical gradient background (deep blue → near black).
    graphics::draw_gradient(
        Color { r: 0x0b, g: 0x12, b: 0x2a },
        Color { r: 0x1a, g: 0x1a, b: 0x2a },
    );

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

/// Cycle through a simple 6-color palette for keystroke echo blocks.
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
