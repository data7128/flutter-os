//! Flutter Shell UI widgets — Material3-inspired desktop components.
//!
//! These are **software-rendered** widgets drawn directly to a pixel
//! buffer. No Flutter Engine, no Skia — just raw pixel manipulation.
//!
//! The design mimics Material3 visual style:
//! - Rounded corners (simplified to sharp corners in skeleton)
//! - Elevation via shadow (simplified to darker border)
//! - Color tokens (primary, surface, on-surface)
//!
//! [MANUAL] The real Flutter Shell would use Flutter Engine + Dart
//! to render Material3 widgets. This is a software-rendered stand-in.

use crate::render::Color;

/// Material3 color palette (simplified).
pub mod m3_colors {
    use super::Color;
    pub const PRIMARY: Color = Color { r: 0x6f, g: 0x45, b: 0xe8 };   // Purple
    pub const ON_PRIMARY: Color = Color { r: 0xff, g: 0xff, b: 0xff };
    pub const SURFACE: Color = Color { r: 0x1c, g: 0x1b, b: 0x1f };
    pub const SURFACE_VARIANT: Color = Color { r: 0x2d, g: 0x2d, b: 0x30 };
    pub const ON_SURFACE: Color = Color { r: 0xe6, g: 0xe1, b: 0xe9 };
    pub const ON_SURFACE_VARIANT: Color = Color { r: 0xc9, g: 0xc4, b: 0xd0 };
    pub const OUTLINE: Color = Color { r: 0x7a, g: 0x76, b: 0x80 };
    pub const ERROR: Color = Color { r: 0xf2, g: 0xb8, b: 0xb5 };
    pub const BACKGROUND: Color = Color { r: 0x14, g: 0x13, b: 0x16 };
}

/// Task bar (bottom of screen) — shows running apps + clock.
pub struct TaskBar {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub clock_text: [u8; 8],
    pub clock_len: usize,
}

impl TaskBar {
    pub const fn new(screen_width: u32, screen_height: u32) -> Self {
        Self {
            x: 0,
            y: screen_height.saturating_sub(48),
            width: screen_width,
            height: 48,
            clock_text: [0; 8],
            clock_len: 0,
        }
    }

    pub fn set_clock(&mut self, text: &[u8]) {
        let len = text.len().min(7);
        self.clock_text[..len].copy_from_slice(&text[..len]);
        self.clock_len = len;
    }

    pub fn draw(&self, render: &crate::render::Renderer) {
        render.fill_rect(self.x, self.y, self.width, self.height, m3_colors::SURFACE_VARIANT);
        render.draw_line(self.x, self.y, self.x + self.width, self.y, m3_colors::OUTLINE);

        // Draw clock placeholder.
        render.fill_rect(
            self.x + self.width - 100,
            self.y + 8,
            80,
            32,
            m3_colors::SURFACE,
        );

        // Draw start button placeholder.
        render.fill_rect(self.x + 8, self.y + 8, 32, 32, m3_colors::PRIMARY);
    }
}

/// Application launcher — full-screen overlay with app icons.
pub struct AppLauncher {
    pub visible: bool,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub apps: [AppEntry; 8],
    pub app_count: usize,
}

#[derive(Clone, Copy)]
pub struct AppEntry {
    pub name: [u8; 16],
    pub name_len: usize,
    pub icon_color: Color,
}

impl AppLauncher {
    pub const fn new(screen_w: u32, screen_h: u32) -> Self {
        Self {
            visible: false,
            x: screen_w / 4,
            y: screen_h / 4,
            width: screen_w / 2,
            height: screen_h / 2,
            apps: [AppEntry { name: [0; 16], name_len: 0, icon_color: Color { r: 0, g: 0, b: 0 } }; 8],
            app_count: 0,
        }
    }

    pub fn add_app(&mut self, name: &[u8], icon_color: Color) {
        if self.app_count >= 8 { return; }
        let len = name.len().min(15);
        let entry = &mut self.apps[self.app_count];
        entry.name[..len].copy_from_slice(&name[..len]);
        entry.name_len = len;
        entry.icon_color = icon_color;
        self.app_count += 1;
    }

    pub fn draw(&self, render: &crate::render::Renderer) {
        if !self.visible { return; }
        render.fill_rect(self.x, self.y, self.width, self.height, m3_colors::SURFACE);
        render.draw_rect_outline(self.x, self.y, self.width, self.height, m3_colors::OUTLINE);

        let icon_size = 48u32;
        let padding = 16u32;
        let cols = (self.width / (icon_size + padding)).max(1);

        for i in 0..self.app_count {
            let col = i as u32 % cols;
            let row = i as u32 / cols;
            let ix = self.x + padding + col * (icon_size + padding);
            let iy = self.y + padding + row * (icon_size + padding);
            render.fill_rect(ix, iy, icon_size, icon_size, self.apps[i].icon_color);
        }
    }
}

/// Clock display — top-right of screen.
pub struct ClockDisplay {
    pub x: u32,
    pub y: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl ClockDisplay {
    pub const fn new(screen_w: u32) -> Self {
        Self { x: screen_w.saturating_sub(80), y: 8, hour: 0, minute: 0, second: 0 }
    }

    pub fn set_time(&mut self, h: u32, m: u32, s: u32) {
        self.hour = h; self.minute = m; self.second = s;
    }

    pub fn draw(&self, render: &crate::render::Renderer) {
        render.fill_rect(self.x, self.y, 72, 24, m3_colors::SURFACE);
        render.draw_rect_outline(self.x, self.y, 72, 24, m3_colors::OUTLINE);

        // Draw time as colored segments (simplified).
        let seg_color = m3_colors::ON_SURFACE;
        let bar_w = 4u32;
        let gap = 2u32;
        let mut cx = self.x + 4;

        // Hours bar (proportional fill).
        let h_fill = (self.hour * 28) / 24;
        render.fill_rect(cx, self.y + 4, h_fill, 16, seg_color);
        cx += 28 + gap;

        // Minutes bar.
        let m_fill = (self.minute * 28) / 60;
        render.fill_rect(cx, self.y + 4, m_fill, 16, seg_color);
        cx += 28 + gap;

        // Seconds bar.
        let s_fill = (self.second * 4) / 60;
        render.fill_rect(cx, self.y + 4, s_fill, 16, m3_colors::PRIMARY);

        let _ = bar_w;
    }
}
