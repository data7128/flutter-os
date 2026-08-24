//! Flutter Shell — AeroOS system desktop (Ring3, no_std).
//!
//! A software-rendered desktop shell that provides:
//! - Material3-inspired task bar
//! - Application launcher
//! - Clock display
//! - WM integration (renders to off-screen buffer, commits via fb_commit)
//!
//! ## Architecture
//!
//! ```text
//! Flutter Shell (Ring3)
//! ├── render.rs      — software pixel renderer
//! ├── widgets.rs     — Material3 widgets (TaskBar, AppLauncher, Clock)
//! ├── syscalls.rs    — kernel syscall wrappers
//! └── lib.rs         — init + event loop
//! ```
//!
//! The Flutter Shell connects to the WM via:
//! - `fb_commit` — submits rendered frames
//! - `poll_input` — receives keyboard/mouse events
//!
//! [MANUAL] The real Flutter Shell would use the Flutter Engine
//! (Dart + Skia) to render Material3 widgets. This is a software
//! fallback for when the Flutter Engine is not yet available.
//!
//! ## Status
//! - Software rendering skeleton: ✅
//! - Material3 widget styling: ✅ (simplified)
//! - Flutter Engine integration: 【暂未实现 — 需要完整引擎】
//! - WM IPC: 【需要人工调试】
//!
//! ## Constraints
//! - Pure software rendering, no GPU acceleration
//! - PS/2 keyboard only, no USB (until Phase 8)
//! - Flutter Engine must never run in kernel space

#![no_std]
#![allow(dead_code)]
#![allow(static_mut_refs)]

pub mod render;
pub mod syscalls;
pub mod widgets;

use render::{Color, Renderer};
use widgets::{AppLauncher, ClockDisplay, TaskBar, m3_colors};
use syscalls::{FramebufferInfo, InputEvent, EVENT_KEYBOARD, EVENT_MOUSE};

/// Shell state.
struct ShellState {
    renderer: Option<Renderer>,
    taskbar: Option<TaskBar>,
    launcher: Option<AppLauncher>,
    clock: Option<ClockDisplay>,
    tick_count: u64,
}

static mut SHELL: ShellState = ShellState {
    renderer: None,
    taskbar: None,
    launcher: None,
    clock: None,
    tick_count: 0,
};

/// Initialise the Flutter Shell.
pub fn init() -> Result<(), i32> {
    let shell = unsafe { &mut SHELL };

    // Query framebuffer info.
    let mut fb_info = FramebufferInfo::default();
    let ret = syscalls::get_framebuffer_info(&mut fb_info);
    if ret < 0 {
        #[cfg(feature = "skeleton")]
        {
            fb_info.width = 1280;
            fb_info.height = 720;
        }
        #[cfg(not(feature = "skeleton"))]
        { return Err(ret as i32); }
    }

    let w = fb_info.width;
    let h = fb_info.height;

    shell.renderer = Some(Renderer::new(w, h));
    shell.taskbar = Some(TaskBar::new(w, h));
    shell.launcher = Some(AppLauncher::new(w, h));
    shell.clock = Some(ClockDisplay::new(w));

    // Add some placeholder apps.
    if let Some(ref mut launcher) = shell.launcher {
        launcher.add_app(b"Terminal", m3_colors::PRIMARY);
        launcher.add_app(b"Files", Color { r: 0x40, g: 0xc4, b: 0xff });
        launcher.add_app(b"Settings", Color { r: 0x66, g: 0x66, b: 0x66 });
    }

    Ok(())
}

/// Main shell event loop — never returns.
///
/// [MANUAL] When Ring3 is ready, this runs as a user-mode process.
pub fn run() -> ! {
    let shell = unsafe { &mut SHELL };

    loop {
        // 1. Poll for input events.
        let mut event = InputEvent::default();
        loop {
            let ret = syscalls::poll_input(&mut event);
            if ret <= 0 { break; }

            match event.event_type {
                EVENT_KEYBOARD => {
                    if event.key_pressed != 0 && event.key_char == b'l' as u32 {
                        // Toggle launcher on 'l' key.
                        if let Some(ref mut launcher) = shell.launcher {
                            launcher.visible = !launcher.visible;
                        }
                    }
                }
                EVENT_MOUSE => {
                    // [MANUAL] Route mouse events to WM.
                }
                _ => {}
            }
        }

        // 2. Render desktop.
        if let Some(ref renderer) = shell.renderer {
            renderer.clear(m3_colors::BACKGROUND);

            // Draw task bar.
            if let Some(ref tb) = shell.taskbar {
                tb.draw(renderer);
            }

            // Draw clock (update time every 60 ticks).
            let t = shell.tick_count;
            let seconds = (t / 60) % 60;
            let minutes = (t / 3600) % 60;
            let hours = ((t / 216000) + 10) % 24; // Offset for demo.
            if let Some(ref mut clock_mut) = unsafe { &mut SHELL }.clock {
                clock_mut.set_time(hours as u32, minutes as u32, seconds as u32);
            }
            if let Some(ref clk) = shell.clock {
                clk.draw(renderer);
            }

            // Draw launcher if visible.
            if let Some(ref launcher) = shell.launcher {
                launcher.draw(renderer);
            }

            // Commit to framebuffer.
            renderer.commit();
        }

        // 3. Increment tick counter.
        shell.tick_count += 1;

        // 4. Sleep ~1ms.
        syscalls::nanosleep(1);
    }
}
