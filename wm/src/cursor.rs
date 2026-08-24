//! Software mouse cursor rendering.
//!
//! Draws a simple arrow-shaped cursor directly on the off-screen
//! buffer, on top of all windows. No hardware cursor support.
//!
//! **Constraint**: single mouse only — no multi-cursor support.

use crate::compositor::Compositor;
use crate::window::Color;

/// Cursor hotspot: the pixel that represents the "click point"
/// relative to the top-left of the cursor bitmap.
pub const CURSOR_HOTSPOT_X: i32 = 0;
pub const CURSOR_HOTSPOT_Y: i32 = 0;

/// Cursor bitmap dimensions.
pub const CURSOR_WIDTH: usize = 11;
pub const CURSOR_HEIGHT: usize = 16;

/// Arrow cursor bitmap (1 = foreground, 0 = transparent).
///
/// ```text
/// X..........
/// XX.........
/// X.X........
/// X..X.......
/// X...X......
/// X....X.....
/// X.....X....
/// X......X...
/// X.......X..
/// X........X.
/// X.........X
/// X.....XXXX.
/// X..XX.......
/// X.XX........
/// XX..........
/// X...........
/// ```
const CURSOR_BITMAP: [[u8; CURSOR_WIDTH]; CURSOR_HEIGHT] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0],
    [1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0],
    [1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];

/// Mouse cursor state: position, button state, visibility.
pub struct Cursor {
    /// Current X position (in framebuffer coordinates).
    pub x: i32,
    /// Current Y position (in framebuffer coordinates).
    pub y: i32,
    /// Button state: bit 0 = left, bit 1 = right, bit 2 = middle.
    pub buttons: u8,
    /// Whether the cursor is visible.
    pub visible: bool,
}

impl Cursor {
    pub const fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            buttons: 0,
            visible: true,
        }
    }

    /// Apply a mouse delta from the input event.
    pub fn apply_delta(&mut self, dx: i16, dy: i16, buttons: u8, fb_width: u32, fb_height: u32) {
        self.x = (self.x + dx as i32).max(0).min(fb_width as i32 - 1);
        self.y = (self.y + dy as i32).max(0).min(fb_height as i32 - 1);
        self.buttons = buttons;
    }

    /// Set the cursor position directly.
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    /// Is the left button currently pressed?
    pub fn left_button(&self) -> bool {
        (self.buttons & 0x01) != 0
    }

    /// Is the right button currently pressed?
    pub fn right_button(&self) -> bool {
        (self.buttons & 0x02) != 0
    }

    /// Draw the cursor on the compositor's off-screen buffer.
    ///
    /// Draws on top of everything — called after `composite()`.
    pub fn draw(&self, comp: &Compositor) {
        if !self.visible {
            return;
        }

        let base_x = self.x - CURSOR_HOTSPOT_X;
        let base_y = self.y - CURSOR_HOTSPOT_Y;

        for row in 0..CURSOR_HEIGHT {
            for col in 0..CURSOR_WIDTH {
                if CURSOR_BITMAP[row][col] == 1 {
                    // Draw with a white pixel with black outline effect:
                    // if adjacent pixel is 0, draw black first for contrast.
                    let px = base_x + col as i32;
                    let py = base_y + row as i32;

                    // Black outline (draw slightly larger).
                    if row == 0 || col == 0
                        || row == CURSOR_HEIGHT - 1
                        || col == CURSOR_WIDTH - 1
                    {
                        comp.put_pixel(px, py, Color::BLACK);
                    } else {
                        comp.put_pixel(px, py, Color::WHITE);
                    }
                }
            }
        }

        // Draw hotspot pixel as a bright marker.
        comp.put_pixel(self.x, self.y, Color { r: 0xff, g: 0x00, b: 0xff });
    }

    /// Erase the cursor region (called before recomposition to
    /// avoid cursor artifacts in the off-screen buffer).
    ///
    /// Actually, since we always re-composite the full screen before
    /// drawing the cursor, we don't need a separate erase step.
    /// The cursor is drawn last and the next composite overwrites it.
    pub fn erase(&self, _comp: &Compositor) {
        // No-op: full-screen recomposition handles this.
    }
}
