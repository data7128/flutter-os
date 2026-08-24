//! Software rendering compositor.
//!
//! Iterates windows in Z-order (bottom → top), clips each window to
//! the visible screen area, blits its content buffer to the off-screen
//! render buffer, draws title bars and borders, then draws the desktop
//! background for any uncovered area.
//!
//! After composition, calls `fb_commit` to blit the off-screen buffer
//! to the physical framebuffer.
//!
//! **Pure software rendering** — no hardware acceleration, no GPU.
//! All pixel operations use CPU `memcpy`/`copy_nonoverlapping`.

use crate::syscalls::{self, FramebufferInfo};
use crate::window::{Color, Window, WindowList, BORDER_WIDTH, TITLE_BAR_HEIGHT};

/// Maximum framebuffer size supported by the off-screen buffer.
pub const MAX_FB_WIDTH: usize = 1280;
pub const MAX_FB_HEIGHT: usize = 720;
pub const BPP: usize = 4;

/// Off-screen render buffer (double buffering).
///
/// [MANUAL] In Ring3, this would be allocated via `mmap` syscall.
/// For the skeleton, we use a static array.
static mut OFFSCREEN_BUFFER: [u8; MAX_FB_WIDTH * MAX_FB_HEIGHT * BPP] =
    [0; MAX_FB_WIDTH * MAX_FB_HEIGHT * BPP];

/// Compositor state — holds framebuffer geometry and dirty region.
pub struct Compositor {
    fb_info: FramebufferInfo,
    /// Whether the compositor has been initialised.
    initialised: bool,
}

impl Compositor {
    pub const fn new() -> Self {
        Self {
            fb_info: FramebufferInfo {
                address: 0,
                width: 0,
                height: 0,
                stride: 0,
                bytes_per_pixel: 0,
                format: 0,
                len: 0,
            },
            initialised: false,
        }
    }

    /// Initialise: query framebuffer info from kernel.
    pub fn init(&mut self) -> Result<(), i32> {
        let ret = syscalls::get_framebuffer_info(&mut self.fb_info);
        if ret < 0 {
            #[cfg(feature = "skeleton")]
            {
                self.fb_info = FramebufferInfo {
                    address: 0xFD000000,
                    width: 1280,
                    height: 720,
                    stride: 1280,
                    bytes_per_pixel: 4,
                    format: 1,
                    len: (1280 * 720 * 4) as u64,
                };
            }
            #[cfg(not(feature = "skeleton"))]
            {
                return Err(ret as i32);
            }
        }
        self.initialised = true;
        Ok(())
    }

    /// Width of the framebuffer in pixels.
    pub fn width(&self) -> u32 {
        self.fb_info.width
    }

    /// Height of the framebuffer in pixels.
    pub fn height(&self) -> u32 {
        self.fb_info.height
    }

    /// Get a reference to the off-screen buffer slice.
    fn offscreen(&self) -> &[u8] {
        unsafe { &OFFSCREEN_BUFFER }
    }

    /// Get a mutable reference to the off-screen buffer slice.
    fn offscreen_mut(&self) -> &mut [u8] {
        unsafe { &mut OFFSCREEN_BUFFER }
    }

    /// Write a single pixel to the off-screen buffer at (x, y).
    pub fn put_pixel(&self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 {
            return;
        }
        let x = x as usize;
        let y = y as usize;
        let w = self.fb_info.width as usize;
        let h = self.fb_info.height as usize;
        if x >= w || y >= h {
            return;
        }
        let offset = (y * w + x) * BPP;
        let buf = self.offscreen_mut();
        if offset + BPP > buf.len() {
            return;
        }
        // Write as BGRA/XRGB depending on format. For simplicity,
        // we always write RGBX and let the fb_commit handle it.
        buf[offset] = color.b; // B
        buf[offset + 1] = color.g; // G
        buf[offset + 2] = color.r; // R
        buf[offset + 3] = 0; // X (padding)
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        let x_end = x.saturating_add(w as i32);
        let y_end = y.saturating_add(h as i32);
        for py in y..y_end {
            for px in x..x_end {
                self.put_pixel(px, py, color);
            }
        }
    }

    /// Draw a 1-pixel rectangle outline.
    pub fn draw_rect_outline(&self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        // Top and bottom edges.
        self.fill_rect(x, y, w, 1, color);
        self.fill_rect(x, y + h as i32 - 1, w, 1, color);
        // Left and right edges.
        self.fill_rect(x, y, 1, h, color);
        self.fill_rect(x + w as i32 - 1, y, 1, h, color);
    }

    /// Draw a window's title bar text (simple bitmap font — just the
    /// title bytes as colored pixels for now).
    ///
    /// [MANUAL] Real text rendering needs a font system (FreeType or
    /// a built-in bitmap font). This is a placeholder.
    fn draw_title_text(&self, win: &Window) {
        // Draw a small colored square as a "logo" in the title bar.
        let logo_color = if win.focused {
            Color { r: 0xff, g: 0xbf, b: 0x00 }
        } else {
            Color { r: 0x80, g: 0x80, b: 0x80 }
        };
        self.fill_rect(win.x + 4, win.y + 6, 12, 12, logo_color);
    }

    /// Composite a single window: draw border, title bar, content.
    fn composite_window(&self, win: &Window) {
        if !win.visible {
            return;
        }

        // 1. Draw window border.
        let border_color = if win.focused {
            Color::BORDER
        } else {
            Color { r: 0x22, g: 0x2a, b: 0x35 }
        };
        self.draw_rect_outline(
            win.x,
            win.y,
            win.full_width(),
            win.full_height(),
            border_color,
        );

        // 2. Draw title bar background.
        let title_color = if win.focused {
            Color::TITLE_BAR_ACTIVE
        } else {
            Color::TITLE_BAR
        };
        self.fill_rect(
            win.x + 1,
            win.y + 1,
            win.full_width() - 2,
            TITLE_BAR_HEIGHT,
            title_color,
        );

        // 3. Draw title text (placeholder).
        self.draw_title_text(win);

        // 4. Blit window content buffer (if provided).
        //
        // The content buffer is `width * height * bpp` bytes,
        // laid out row-major. We clip it to the window's content area.
        if win.content_buf.is_null() || win.content_len == 0 {
            // No content — fill with dark background.
            self.fill_rect(
                win.x + BORDER_WIDTH as i32,
                win.y + TITLE_BAR_HEIGHT as i32,
                win.width,
                win.height,
                Color::DARK_BG,
            );
            return;
        }

        // Blit content, clipping to screen bounds.
        let content_x = win.x + BORDER_WIDTH as i32;
        let content_y = win.y + TITLE_BAR_HEIGHT as i32;
        let fb_w = self.fb_info.width as i32;
        let fb_h = self.fb_info.height as i32;
        let buf = unsafe {
            core::slice::from_raw_parts(win.content_buf, win.content_len)
        };

        for row in 0..win.height as i32 {
            let py = content_y + row;
            if py < 0 || py >= fb_h {
                continue;
            }
            for col in 0..win.width as i32 {
                let px = content_x + col;
                if px < 0 || px >= fb_w {
                    continue;
                }
                let src_off = (row as usize * win.width as usize + col as usize) * BPP;
                if src_off + BPP > buf.len() {
                    break;
                }
                // Copy pixel directly (assuming same format).
                let offscreen = self.offscreen_mut();
                let dst_off = (py as usize * fb_w as usize + px as usize) * BPP;
                if dst_off + BPP <= offscreen.len() && src_off + BPP <= buf.len() {
                    offscreen[dst_off] = buf[src_off]; // B
                    offscreen[dst_off + 1] = buf[src_off + 1]; // G
                    offscreen[dst_off + 2] = buf[src_off + 2]; // R
                    offscreen[dst_off + 3] = buf[src_off + 3]; // X
                }
            }
        }
    }

    /// Composite the entire screen: desktop background + all windows.
    ///
    /// 1. Fill desktop background.
    /// 2. Iterate windows Z-order bottom→top, composite each.
    /// 3. The cursor is drawn separately after composition.
    pub fn composite(&self, windows: &WindowList) {
        // 1. Clear to desktop background.
        self.fill_rect(
            0,
            0,
            self.fb_info.width,
            self.fb_info.height,
            Color::DESKTOP,
        );

        // 2. Composite windows in Z-order.
        for win in windows.iter_z_order() {
            self.composite_window(win);
        }
    }

    /// Commit the off-screen buffer to the physical framebuffer.
    ///
    /// Calls `fb_commit` syscall to blit the entire off-screen buffer
    /// to the real framebuffer.
    pub fn commit(&self) -> i64 {
        let buf = self.offscreen();
        syscalls::fb_commit(buf, 0, 0, self.fb_info.width, self.fb_info.height)
    }
}
