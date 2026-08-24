//! Reserved Flutter application surface interface.
//!
//! This module defines the API that a future Flutter application
//! will use to connect to the WM and provide its rendered output.
//!
//! The architecture is:
//!
//! ```text
//! Flutter Engine (Ring3)
//!     │ renders Dart UI to a pixel buffer
//!     ▼
//! FlutterSurface (this module)
//!     │ provides the buffer pointer to the WM
//!     ▼
//! WindowManager::create_flutter_window()
//!     │ creates a Window with the surface as content
//!     ▼
//! Compositor
//!     │ blits the surface content to the off-screen buffer
//!     ▼
//! fb_commit → physical framebuffer
//! ```
//!
//! [MANUAL] All functions here are skeleton only. The real
//! implementation requires:
//! - Ring3 user-mode to be functional
//! - IPC mechanism (shared memory) between Flutter app and WM
//! - Flutter Engine to actually render content

use crate::window::{Color, WindowList};

/// A surface provided by a Flutter application.
///
/// The Flutter Engine renders its Dart UI into this buffer,
/// and the WM composites it onto the screen.
///
/// In a real system, this buffer is shared memory (mmap'd) between
/// the Flutter process and the WM process.
#[derive(Clone, Copy)]
pub struct FlutterSurface {
    /// Pointer to the pixel buffer (BGRA8888 format).
    pub buffer: *mut u8,
    /// Length of the buffer in bytes.
    pub buffer_len: usize,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Whether the buffer has been updated since last composite.
    pub dirty: bool,
}

impl FlutterSurface {
    pub const fn empty() -> Self {
        Self {
            buffer: core::ptr::null_mut(),
            buffer_len: 0,
            width: 0,
            height: 0,
            dirty: false,
        }
    }

    /// Register a Flutter surface as a new window.
    ///
    /// [MANUAL] The real implementation needs IPC to receive the
    /// buffer pointer from the Flutter process.
    pub fn register(
        windows: &mut WindowList,
        x: i32,
        y: i32,
        surface: &FlutterSurface,
        title: &[u8],
    ) -> u32 {
        windows.create(
            x,
            y,
            surface.width,
            surface.height,
            title,
            surface.buffer,
            surface.buffer_len,
        )
    }

    /// Update the surface content (mark dirty).
    ///
    /// The Flutter app calls this after rendering a new frame.
    /// The WM will re-composite on the next event loop iteration.
    ///
    /// [MANUAL] The real implementation uses IPC (pipe or shared
    /// memory notification) to tell the WM the surface is dirty.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clear the dirty flag (called by compositor after blitting).
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}

/// A simple test window content for skeleton mode.
///
/// Fills a buffer with a solid color, used when no real app
/// is connected. This lets the WM demonstrate window rendering
/// without a Flutter Engine.
pub fn fill_test_content(buf: &mut [u8], width: u32, height: u32, color: Color) {
    let bpp = 4usize;
    let row_bytes = width as usize * bpp;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let offset = y * row_bytes + x * bpp;
            if offset + bpp <= buf.len() {
                buf[offset] = color.b;     // B
                buf[offset + 1] = color.g; // G
                buf[offset + 2] = color.r; // R
                buf[offset + 3] = 0;       // X
            }
        }
    }
}
