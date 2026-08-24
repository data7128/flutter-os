//! Window structures and Z-order management.
//!
//! A `Window` has position, size, z-order, a content buffer pointer,
//! and a title bar. The `WindowManager` maintains a fixed-capacity list
//! of windows sorted by Z-order.
//!
//! All GUI logic is here — the kernel has zero window awareness.

/// Maximum windows the WM can manage simultaneously.
pub const MAX_WINDOWS: usize = 16;

/// Title bar height in pixels.
pub const TITLE_BAR_HEIGHT: u32 = 24;

/// Window border width in pixels.
pub const BORDER_WIDTH: u32 = 2;

/// RGB color (matches framebuffer Color in kernel).
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
    pub const DARK_BG: Color = Color { r: 0x0f, g: 0x17, b: 0x24 };
    pub const TITLE_BAR: Color = Color { r: 0x1e, g: 0x29, b: 0x3b };
    pub const TITLE_BAR_ACTIVE: Color = Color { r: 0x2d, g: 0x5a, b: 0x9e };
    pub const BORDER: Color = Color { r: 0x33, g: 0x44, b: 0x55 };
    pub const DESKTOP: Color = Color { r: 0x0b, g: 0x12, b: 0x2a };
}

/// A single window.
///
/// The content buffer is owned by the application process. In a real
/// system, it's shared memory (mmap'd) between the app and WM.
/// For the skeleton, it's a raw pointer that the compositor reads from.
#[derive(Clone, Copy)]
pub struct Window {
    /// Unique window ID (1-based, 0 = empty slot).
    pub id: u32,
    /// Top-left X position on the framebuffer.
    pub x: i32,
    /// Top-left Y position on the framebuffer.
    pub y: i32,
    /// Width of the window content area (excluding title bar + border).
    pub width: u32,
    /// Height of the window content area (excluding title bar + border).
    pub height: u32,
    /// Z-order layer (higher = on top). Set by `raise_to_top`.
    pub z_order: u32,
    /// Whether the window is visible (rendered by compositor).
    pub visible: bool,
    /// Whether the window has keyboard focus.
    pub focused: bool,
    /// Whether the window can be moved by dragging the title bar.
    pub draggable: bool,
    /// Pointer to the window's content buffer (pixel data).
    /// Format: row-major, `width * height * bpp` bytes.
    /// [MANUAL] In Ring3, this is a user-space shared memory buffer.
    pub content_buf: *mut u8,
    /// Length of the content buffer in bytes.
    pub content_len: usize,
    /// Title text (null-terminated, max 31 chars).
    pub title: [u8; 32],
}

impl Window {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            z_order: 0,
            visible: false,
            focused: false,
            draggable: true,
            content_buf: core::ptr::null_mut(),
            content_len: 0,
            title: [0; 32],
        }
    }

    /// Full width including borders.
    pub fn full_width(&self) -> u32 {
        self.width + 2 * BORDER_WIDTH
    }

    /// Full height including title bar + borders.
    pub fn full_height(&self) -> u32 {
        self.height + TITLE_BAR_HEIGHT + 2 * BORDER_WIDTH
    }

    /// Check if a point (px, py) is inside this window's full bounds.
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        let fx = self.x;
        let fy = self.y;
        let fw = self.full_width() as i32;
        let fh = self.full_height() as i32;
        px >= fx && px < fx + fw && py >= fy && py < fy + fh
    }

    /// Check if a point is in the title bar area.
    pub fn is_in_title_bar(&self, px: i32, py: i32) -> bool {
        let fx = self.x;
        let fy = self.y;
        px >= fx
            && px < fx + self.full_width() as i32
            && py >= fy
            && py < fy + TITLE_BAR_HEIGHT as i32
    }

    /// Set the title from a byte slice.
    pub fn set_title(&mut self, title: &[u8]) {
        let len = title.len().min(31);
        self.title[..len].copy_from_slice(&title[..len]);
        self.title[len] = 0; // null-terminate
    }

    /// Get the title as a byte slice (up to the null terminator).
    pub fn title_bytes(&self) -> &[u8] {
        let end = self.title.iter().position(|&b| b == 0).unwrap_or(32);
        &self.title[..end]
    }
}

/// Fixed-capacity window list. No alloc needed.
pub struct WindowList {
    windows: [Window; MAX_WINDOWS],
    count: usize,
    /// Next window ID to assign.
    next_id: u32,
    /// Next z-order value (always increasing).
    next_z: u32,
}

impl WindowList {
    pub const fn new() -> Self {
        Self {
            windows: [Window::empty(); MAX_WINDOWS],
            count: 0,
            next_id: 1,
            next_z: 1,
        }
    }

    /// Create a new window. Returns the window ID, or 0 on failure.
    pub fn create(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        title: &[u8],
        content_buf: *mut u8,
        content_len: usize,
    ) -> u32 {
        if self.count >= MAX_WINDOWS {
            return 0;
        }

        // Find the first empty slot.
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id == 0 {
                let id = self.next_id;
                self.next_id += 1;
                let z = self.next_z;
                self.next_z += 1;

                self.windows[i] = Window {
                    id,
                    x,
                    y,
                    width,
                    height,
                    z_order: z,
                    visible: true,
                    focused: false,
                    draggable: true,
                    content_buf,
                    content_len,
                    title: [0; 32],
                };
                self.windows[i].set_title(title);
                self.count += 1;
                return id;
            }
        }
        0
    }

    /// Destroy a window by ID.
    pub fn destroy(&mut self, id: u32) -> bool {
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id == id {
                self.windows[i] = Window::empty();
                self.count -= 1;
                return true;
            }
        }
        false
    }

    /// Move a window to a new position.
    pub fn move_window(&mut self, id: u32, x: i32, y: i32) -> bool {
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id == id {
                self.windows[i].x = x;
                self.windows[i].y = y;
                return true;
            }
        }
        false
    }

    /// Raise a window to the top of the Z-order.
    pub fn raise_to_top(&mut self, id: u32) -> bool {
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id == id {
                self.windows[i].z_order = self.next_z;
                self.next_z += 1;
                return true;
            }
        }
        false
    }

    /// Set focus on a window (and clear focus on all others).
    pub fn set_focus(&mut self, id: u32) {
        for i in 0..MAX_WINDOWS {
            self.windows[i].focused = self.windows[i].id == id;
        }
    }

    /// Get the focused window ID (0 if none).
    pub fn focused_id(&self) -> u32 {
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id != 0 && self.windows[i].focused {
                return self.windows[i].id;
            }
        }
        0
    }

    /// Hit test: find the topmost window at (px, py).
    pub fn hit_test(&self, px: i32, py: i32) -> u32 {
        let mut top_z: u32 = 0;
        let mut hit_id: u32 = 0;
        for i in 0..MAX_WINDOWS {
            let w = &self.windows[i];
            if w.id == 0 || !w.visible {
                continue;
            }
            if w.contains_point(px, py) && w.z_order >= top_z {
                top_z = w.z_order;
                hit_id = w.id;
            }
        }
        hit_id
    }

    /// Get a reference to a window by ID.
    pub fn get(&self, id: u32) -> Option<&Window> {
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id == id {
                return Some(&self.windows[i]);
            }
        }
        None
    }

    /// Get a mutable reference to a window by ID.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Window> {
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id == id {
                return Some(&mut self.windows[i]);
            }
        }
        None
    }

    /// Iterate windows in Z-order (bottom to top).
    pub fn iter_z_order(&self) -> ZOrderIter<'_> {
        // Collect indices and sort by z_order.
        let mut indices: [usize; MAX_WINDOWS] = [0; MAX_WINDOWS];
        let mut n = 0;
        for i in 0..MAX_WINDOWS {
            if self.windows[i].id != 0 && self.windows[i].visible {
                indices[n] = i;
                n += 1;
            }
        }
        // Simple insertion sort by z_order (small array).
        for i in 1..n {
            let key = indices[i];
            let key_z = self.windows[key].z_order;
            let mut j = i;
            while j > 0 && self.windows[indices[j - 1]].z_order > key_z {
                indices[j] = indices[j - 1];
                j -= 1;
            }
            indices[j] = key;
        }
        ZOrderIter {
            windows: &self.windows,
            indices: indices,
            count: n,
            pos: 0,
        }
    }
}

/// Iterator over windows in Z-order (bottom to top).
pub struct ZOrderIter<'a> {
    windows: &'a [Window; MAX_WINDOWS],
    indices: [usize; MAX_WINDOWS],
    count: usize,
    pos: usize,
}

impl<'a> Iterator for ZOrderIter<'a> {
    type Item = &'a Window;

    fn next(&mut self) -> Option<&'a Window> {
        if self.pos >= self.count {
            return None;
        }
        let idx = self.indices[self.pos];
        self.pos += 1;
        Some(&self.windows[idx])
    }
}
