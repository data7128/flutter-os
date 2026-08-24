//! Software renderer for Flutter Shell — pixel-level drawing.
//!
//! Pure software rendering, no GPU. Draws to an off-screen buffer
//! and commits via `fb_commit` syscall.
//!
//! [MANUAL] The real Flutter Shell would use Flutter Engine's Skia
//! backend for rendering. This is a minimal software fallback.

use crate::syscalls;

/// RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Off-screen render buffer size.
const MAX_WIDTH: usize = 1280;
const MAX_HEIGHT: usize = 720;
const BPP: usize = 4;

static mut OFFSCREEN: [u8; MAX_WIDTH * MAX_HEIGHT * BPP] =
    [0; MAX_WIDTH * MAX_HEIGHT * BPP];

/// Software renderer.
pub struct Renderer {
    pub width: u32,
    pub height: u32,
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    fn buf_mut(&self) -> &mut [u8] {
        unsafe { &mut OFFSCREEN }
    }

    pub fn put_pixel(&self, x: u32, y: u32, color: Color) {
        if x >= self.width || y >= self.height { return; }
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * BPP;
        let buf = self.buf_mut();
        if offset + BPP > buf.len() { return; }
        buf[offset] = color.b;
        buf[offset + 1] = color.g;
        buf[offset + 2] = color.r;
        buf[offset + 3] = 0;
    }

    pub fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        let x_end = (x + w).min(self.width);
        let y_end = (y + h).min(self.height);
        for py in y..y_end {
            for px in x..x_end {
                self.put_pixel(px, py, color);
            }
        }
    }

    pub fn draw_rect_outline(&self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        self.fill_rect(x, y, w, 1, color);
        self.fill_rect(x, y + h - 1, w, 1, color);
        self.fill_rect(x, y, 1, h, color);
        self.fill_rect(x + w - 1, y, 1, h, color);
    }

    pub fn draw_line(&self, x0: u32, y0: u32, x1: u32, y1: u32, color: Color) {
        // Bresenham's line algorithm.
        let dx = (x1 as i64 - x0 as i64).abs();
        let dy = (y1 as i64 - y0 as i64).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx - dy;
        let mut cx = x0 as i64;
        let mut cy = y0 as i64;

        loop {
            self.put_pixel(cx as u32, cy as u32, color);
            if cx == x1 as i64 && cy == y1 as i64 { break; }
            let e2 = 2 * err;
            if e2 > -dy { err -= dy; cx += sx; }
            if e2 < dx { err += dx; cy += sy; }
        }
    }

    pub fn clear(&self, color: Color) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }

    pub fn commit(&self) -> i64 {
        let buf_len = (self.width as usize) * (self.height as usize) * BPP;
        let buf = unsafe { &OFFSCREEN[..buf_len] };
        syscalls::fb_commit(buf, 0, 0, self.width, self.height)
    }
}
