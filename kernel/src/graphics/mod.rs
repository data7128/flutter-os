//! Linear framebuffer abstraction.
//!
//! The bootloader hands us a pixel framebuffer (`BootInfo.framebuffer`). We
//! take ownership of its byte slice with a `'static` lifetime, store the
//! layout, and expose `put_pixel` / `fill_rect` / `clear` for the shell host
//! (and, ultimately, the Flutter engine embedder) to render the desktop.

use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use bootloader_api::BootInfo;
use spin::Mutex;

/// Backend holding the live framebuffer (as a raw pointer) + its layout.
/// Stored behind a spinlock so interrupt handlers and the shell host do not
/// race on the buffer.
pub struct Framebuffer {
    buffer: *mut u8,
    byte_len: usize,
    info: FrameBufferInfo,
}

// The framebuffer is only touched through the `Mutex<Framebuffer>` static,
// and the underlying memory is exclusive to this one reference handed over
// by the bootloader. It is therefore sound to mark it `Send`.
unsafe impl Send for Framebuffer {}

/// The single global framebuffer.
pub static FRAMEBUFFER: Mutex<Option<Framebuffer>> = Mutex::new(None);

/// RGB triplet (0..=255).
#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0x00, g: 0x00, b: 0x00 };
    pub const WHITE: Color = Color { r: 0xff, g: 0xff, b: 0xff };
    pub const AERO: Color = Color { r: 0x3b, g: 0x82, b: 0xf6 }; // AeroOS blue
    pub const GREEN: Color = Color { r: 0x22, g: 0xc5, b: 0x5e };
    pub const AMBER: Color = Color { r: 0xf5, g: 0x9e, b: 0x0b };
    pub const PINK: Color = Color { r: 0xec, g: 0x48, b: 0x99 };
    pub const VIOLET: Color = Color { r: 0x8b, g: 0x5c, b: 0xf6 };
}

/// Take the framebuffer out of `BootInfo` and install it globally.
pub fn init(boot_info: &mut BootInfo) {
    let fb = boot_info
        .framebuffer
        .take()
        .expect("[graphics] no framebuffer (boot with a graphical display)");

    let info = fb.info();
    let buffer: &'static mut [u8] = fb.into_buffer();

    *FRAMEBUFFER.lock() = Some(Framebuffer {
        buffer: buffer.as_mut_ptr(),
        byte_len: buffer.len(),
        info,
    });
}

/// Width of the framebuffer in pixels (0 if not initialised).
pub fn width() -> usize {
    FRAMEBUFFER.lock().as_ref().map(|fb| fb.info.width).unwrap_or(0)
}

/// Height of the framebuffer in pixels (0 if not initialised).
pub fn height() -> usize {
    FRAMEBUFFER.lock().as_ref().map(|fb| fb.info.height).unwrap_or(0)
}

/// Set a single pixel. Silently drops pixels outside the screen.
pub fn put_pixel(x: usize, y: usize, color: Color) {
    let mut guard = FRAMEBUFFER.lock();
    let Some(fb) = guard.as_mut() else { return };
    let info = fb.info;
    if x >= info.width || y >= info.height {
        return;
    }
    let off = (y * info.stride + x) * info.bytes_per_pixel;
    if off + info.bytes_per_pixel > fb.byte_len {
        return;
    }
    unsafe {
        write_pixel(fb.buffer.add(off), info.bytes_per_pixel, info.pixel_format, color);
    }
}

/// Fill an axis-aligned rectangle.
pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, color: Color) {
    let mut guard = FRAMEBUFFER.lock();
    let Some(fb) = guard.as_mut() else { return };
    let info = fb.info;
    for yy in 0..h {
        let py = y + yy;
        if py >= info.height {
            break;
        }
        for xx in 0..w {
            let px = x + xx;
            if px >= info.width {
                break;
            }
            let off = (py * info.stride + px) * info.bytes_per_pixel;
            if off + info.bytes_per_pixel > fb.byte_len {
                continue;
            }
            unsafe {
                write_pixel(
                    fb.buffer.add(off),
                    info.bytes_per_pixel,
                    info.pixel_format,
                    color,
                );
            }
        }
    }
}

/// Clear the whole screen to one color.
pub fn clear(color: Color) {
    let (w, h) = (width(), height());
    fill_rect(0, 0, w, h, color);
}

/// Write the bytes of one pixel to `ptr`, honouring the pixel format.
///
/// # Safety
/// `ptr` must point to at least `bpp` writable bytes.
unsafe fn write_pixel(ptr: *mut u8, bpp: usize, fmt: PixelFormat, color: Color) {
    match fmt {
        PixelFormat::Rgb => {
            *ptr.add(0) = color.r;
            *ptr.add(1) = color.g;
            *ptr.add(2) = color.b;
        }
        PixelFormat::Bgr => {
            *ptr.add(0) = color.b;
            *ptr.add(1) = color.g;
            *ptr.add(2) = color.r;
        }
        _ => {
            // Grayscale / unknown: store a simple luminance value.
            let luma = ((color.r as u16 + color.g as u16 + color.b as u16) / 3) as u8;
            *ptr.add(0) = luma;
        }
    }
    if bpp >= 4 {
        *ptr.add(3) = 0xff;
    }
}
