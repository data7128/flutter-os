//! Framebuffer graphics library — low-level 2D drawing primitives.
//!
//! Provides draw_pixel, fill_rect, and gradient rendering on top of the
//! bootloader-provided linear framebuffer. This is the **graphics
//! abstraction layer** that a future GUI engine (e.g. Flutter embedder)
//! would render onto.
//!
//! ## Future GUI integration point
//!
//! The `init()` function stores the raw framebuffer pointer. A future
//! Flutter engine embedder would call `buffer_mut()` to obtain the
//! `&'static mut [u8]` slice and blit rendered frames directly.

use core::ptr;
use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use spin::Mutex;

/// RGB color value (no alpha — the framebuffer is opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0x00, g: 0x00, b: 0x00 };
    pub const WHITE: Color = Color { r: 0xff, g: 0xff, b: 0xff };
    /// AeroOS brand blue.
    pub const AERO: Color = Color { r: 0x1d, g: 0x4e, b: 0x89 };
    pub const GREEN: Color = Color { r: 0x00, g: 0xff, b: 0x00 };
    pub const AMBER: Color = Color { r: 0xff, g: 0xbf, b: 0x00 };
    pub const PINK: Color = Color { r: 0xff, g: 0x69, b: 0xb4 };
    pub const VIOLET: Color = Color { r: 0x8a, g: 0x2b, b: 0xe2 };
    pub const DARK_BG: Color = Color { r: 0x0b, g: 0x12, b: 0x2a };
}

/// Internal graphics state: framebuffer info + raw pointer.
struct GraphicsState {
    info: Option<FrameBufferInfo>,
    buffer: *mut u8,
    len: usize,
}

// The framebuffer is a single global region accessible from any context
// (including interrupt handlers). On single-core bare metal this is safe.
unsafe impl Send for GraphicsState {}

static GRAPHICS: Mutex<GraphicsState> = Mutex::new(GraphicsState {
    info: None,
    buffer: core::ptr::null_mut(),
    len: 0,
});

/// Initialise the graphics subsystem from the bootloader-provided framebuffer.
///
/// ## Future GUI integration
///
/// After this call, the framebuffer is ready for rendering. A Flutter engine
/// embedder would call `width()`, `height()`, and `buffer_mut()` to blit
/// composed Dart UI frames.
pub fn init(framebuffer: FrameBuffer) {
    let info = framebuffer.info();
    // `into_buffer` consumes the FrameBuffer and returns a &'static mut [u8]
    // pointing at the mapped framebuffer memory.
    let buffer = framebuffer.into_buffer();
    let ptr = buffer.as_mut_ptr();
    let len = buffer.len();

    let mut state = GRAPHICS.lock();
    state.info = Some(info);
    state.buffer = ptr;
    state.len = len;

    crate::serial::_print(format_args!(
        "[graphics] framebuffer {}x{} ({} bpp, {:?})\n",
        info.width, info.height, info.bytes_per_pixel, info.pixel_format
    ));
}

/// Whether the graphics subsystem has been initialised.
pub fn is_ready() -> bool {
    GRAPHICS.lock().info.is_some()
}

/// Framebuffer width in pixels.
pub fn width() -> usize {
    GRAPHICS.lock().info.map(|i| i.width).unwrap_or(0)
}

/// Framebuffer height in pixels.
pub fn height() -> usize {
    GRAPHICS.lock().info.map(|i| i.height).unwrap_or(0)
}

/// Bytes per pixel (typically 3 for BGR, 4 for RGBX).
pub fn bytes_per_pixel() -> usize {
    GRAPHICS.lock().info.map(|i| i.bytes_per_pixel).unwrap_or(0)
}

/// Write a single pixel at (x, y). Out-of-bounds writes are silently
/// dropped.
pub fn draw_pixel(x: usize, y: usize, color: Color) {
    let state = GRAPHICS.lock();
    let Some(info) = state.info else {
        return;
    };
    if x >= info.width || y >= info.height {
        return;
    }
    let offset = (y * info.stride + x) * info.bytes_per_pixel;
    if offset + info.bytes_per_pixel > state.len {
        return;
    }
    unsafe {
        let ptr = state.buffer.add(offset);
        match info.pixel_format {
            PixelFormat::Rgb => {
                ptr::write_volatile(ptr, color.r);
                ptr::write_volatile(ptr.add(1), color.g);
                ptr::write_volatile(ptr.add(2), color.b);
            }
            PixelFormat::Bgr => {
                ptr::write_volatile(ptr, color.b);
                ptr::write_volatile(ptr.add(1), color.g);
                ptr::write_volatile(ptr.add(2), color.r);
            }
            _ => {
                // Unknown format: write RGB bytes as a best-effort.
                ptr::write_volatile(ptr, color.r);
                ptr::write_volatile(ptr.add(1), color.g);
                ptr::write_volatile(ptr.add(2), color.b);
            }
        }
    }
}

/// Fill a rectangle with a solid color.
pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, color: Color) {
    for dy in 0..h {
        for dx in 0..w {
            draw_pixel(x + dx, y + dy, color);
        }
    }
}

/// Draw a vertical gradient from `top` color (at y=0) to `bottom` color
/// (at y=height-1). Uses integer math only (no FPU).
pub fn draw_gradient(top: Color, bottom: Color) {
    let h = height();
    let w = width();
    if h == 0 || w == 0 {
        return;
    }
    for y in 0..h {
        // t = y * 256 / h, range 0..256 (fixed-point 8.8)
        let t = (y as u32).wrapping_mul(256) / h as u32;
        let inv_t = 256 - t;
        let r = (top.r as u32 * inv_t + bottom.r as u32 * t) / 256;
        let g = (top.g as u32 * inv_t + bottom.g as u32 * t) / 256;
        let b = (top.b as u32 * inv_t + bottom.b as u32 * t) / 256;
        let color = Color {
            r: r as u8,
            g: g as u8,
            b: b as u8,
        };
        for x in 0..w {
            draw_pixel(x, y, color);
        }
    }
}

/// Clear the entire framebuffer to a single color.
pub fn clear(color: Color) {
    fill_rect(0, 0, width(), height(), color);
}


