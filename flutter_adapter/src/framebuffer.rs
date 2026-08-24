//! Framebuffer adapter — maps the kernel framebuffer via mmap and
//! provides a memory canvas for the Flutter Engine to render into.
//!
//! ## Architecture
//!
//! ```text
//! Kernel framebuffer (physical)
//!     ↓ mmap syscall
//! User-mode virtual address (mapped region)
//!     ↓
//! FramebufferCanvas (this module)
//!     ├── width(), height(), stride(), bpp()
//!     ├── put_pixel(x, y, color)
//!     ├── clear(color)
//!     └── blit(buf, x, y, w, h) — for Skia/Impeller output
//! ```
//!
//! ## Usage
//!
//! 1. `init()` — calls `get_framebuffer_info` + `mmap` to obtain
//!    a writable slice to the framebuffer.
//! 2. `canvas().put_pixel(x, y, color)` — direct pixel writes.
//! 3. `canvas().blit(buf, x, y, w, h)` — blit a rendered frame
//!    from the Flutter Engine's raster output.
//!
//! [MANUAL] When Ring3 is implemented, mmap will return a real
//! user-accessible virtual mapping of the framebuffer. Currently
//! this is a skeleton that doesn't actually map anything.

use crate::syscalls::{self, FramebufferInfo};

/// RGB color (matches kernel graphics::Color).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color { r: 0xff, g: 0xff, b: 0xff };
    pub const AERO: Color = Color { r: 0x1d, g: 0x4e, b: 0x89 };
}

/// Pixel format of the mapped framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    Bgr,
    Unknown,
}

/// The framebuffer canvas — a thin wrapper over the mmap'd memory.
pub struct FramebufferCanvas {
    /// Pointer to the mapped framebuffer memory.
    buffer: *mut u8,
    /// Total length in bytes.
    len: usize,
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
    /// Stride (pixels per row).
    stride: u32,
    /// Bytes per pixel.
    bpp: u32,
    /// Pixel byte order.
    format: PixelFormat,
}

// The framebuffer pointer is safe to Send on single-core.
unsafe impl Send for FramebufferCanvas {}

impl FramebufferCanvas {
    /// Create a canvas from raw components.
    pub const fn new(
        buffer: *mut u8,
        len: usize,
        width: u32,
        height: u32,
        stride: u32,
        bpp: u32,
        format: PixelFormat,
    ) -> Self {
        Self {
            buffer,
            len,
            width,
            height,
            stride,
            bpp,
            format,
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per pixel.
    pub fn bpp(&self) -> u32 {
        self.bpp
    }

    /// Write a single pixel at (x, y).
    pub fn put_pixel(&self, x: u32, y: u32, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = ((y * self.stride + x) * self.bpp) as usize;
        if offset + self.bpp as usize > self.len {
            return;
        }
        unsafe {
            let ptr = self.buffer.add(offset);
            match self.format {
                PixelFormat::Rgb => {
                    core::ptr::write_volatile(ptr, color.r);
                    core::ptr::write_volatile(ptr.add(1), color.g);
                    core::ptr::write_volatile(ptr.add(2), color.b);
                }
                PixelFormat::Bgr => {
                    core::ptr::write_volatile(ptr, color.b);
                    core::ptr::write_volatile(ptr.add(1), color.g);
                    core::ptr::write_volatile(ptr.add(2), color.r);
                }
                PixelFormat::Unknown => {
                    core::ptr::write_volatile(ptr, color.r);
                    core::ptr::write_volatile(ptr.add(1), color.g);
                    core::ptr::write_volatile(ptr.add(2), color.b);
                }
            }
        }
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        for dy in 0..h {
            for dx in 0..w {
                self.put_pixel(x + dx, y + dy, color);
            }
        }
    }

    /// Clear the entire framebuffer to a color.
    pub fn clear(&self, color: Color) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }

    /// Blit a raw pixel buffer to the framebuffer.
    ///
    /// `src` is a slice of RGB/BGR bytes (matching `self.format`).
    /// `src_width` and `src_height` specify the dimensions.
    /// The blit is placed at (dest_x, dest_y) on the framebuffer.
    ///
    /// [MANUAL] This is the integration point where Flutter Engine's
    /// rasteriser output (from Skia or Impeller) gets written to the
    /// screen. The engine produces a buffer of pixels, and this
    /// function copies them into the mmap'd framebuffer.
    pub fn blit(&self, src: &[u8], src_width: u32, src_height: u32, dest_x: u32, dest_y: u32) {
        for y in 0..src_height {
            let fb_y = dest_y + y;
            if fb_y >= self.height {
                break;
            }
            for x in 0..src_width {
                let fb_x = dest_x + x;
                if fb_x >= self.width {
                    break;
                }
                let src_offset = ((y * src_width + x) * self.bpp) as usize;
                if src_offset + self.bpp as usize > src.len() {
                    return;
                }
                let r = src[src_offset];
                let g = src[src_offset + 1];
                let b = src[src_offset + 2];
                self.put_pixel(fb_x, fb_y, Color { r, g, b });
            }
        }
    }
}

/// Global framebuffer canvas state.
struct FbState {
    canvas: Option<FramebufferCanvas>,
}

static mut FB_STATE: FbState = FbState { canvas: None };

/// Initialise the framebuffer adapter.
///
/// 1. Query framebuffer info from kernel via `get_framebuffer_info`
/// 2. `mmap` the framebuffer region
/// 3. Create a `FramebufferCanvas` wrapper
///
/// [MANUAL] Steps 1-2 require real Ring3 syscall support. Currently
/// returns `Err(ENOSYS)`.
pub fn init() -> Result<(), i32> {
    // 1. Query framebuffer geometry.
    let mut info = FramebufferInfo {
        address: 0,
        width: 0,
        height: 0,
        stride: 0,
        bytes_per_pixel: 0,
        format: 0,
    };
    let ret = syscalls::get_framebuffer_info(&mut info);
    if ret < 0 {
        // [MANUAL] SYS_GET_FB_INFO not yet implemented in kernel.
        // SKELETON: use a fallback 1280x720 BGR config for testing.
        #[cfg(feature = "skeleton")]
        {
            info.address = 0xFD000000; // QEMU Bochs VBE default
            info.width = 1280;
            info.height = 720;
            info.stride = 1280;
            info.bytes_per_pixel = 4;
            info.format = 1; // BGR
        }
        #[cfg(not(feature = "skeleton"))]
        return Err(ret as i32);
    }

    // 2. Map the framebuffer memory.
    let fb_len = (info.stride * info.height * info.bytes_per_pixel) as u64;
    let ptr = syscalls::mmap(info.address, fb_len, 0x7); // PROT_READ|WRITE|EXEC
    if ptr < 0 {
        return Err(ptr as i32);
    }

    // 3. Create canvas.
    let format = match info.format {
        0 => PixelFormat::Rgb,
        1 => PixelFormat::Bgr,
        _ => PixelFormat::Unknown,
    };

    let canvas = FramebufferCanvas::new(
        ptr as *mut u8,
        fb_len as usize,
        info.width,
        info.height,
        info.stride,
        info.bytes_per_pixel,
        format,
    );

    // SAFETY: single-threaded skeleton.
    unsafe {
        FB_STATE.canvas = Some(canvas);
    }

    Ok(())
}

/// Get a reference to the framebuffer canvas.
pub fn canvas() -> &'static FramebufferCanvas {
    // SAFETY: single-threaded skeleton. In a real system, this would
    // return a reference to the mapped framebuffer.
    unsafe {
        FB_STATE.canvas.as_ref().expect("framebuffer not initialised")
    }
}
