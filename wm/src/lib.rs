//! AeroOS User-Mode Window Manager (Ring3, no_std).
//!
//! This crate implements a software-rendering window compositor that
//! runs as a **Ring3 user-mode process** — never in kernel space.
//! The kernel provides two new syscalls:
//!
//! - `SYS_FB_COMMIT` — blit a buffer to the physical framebuffer
//! - `SYS_POLL_INPUT` — poll for structured keyboard/mouse events
//!
//! ## Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────┐
//! │  Application Processes (Ring3)                        │
//! │  ┌────────┐  ┌────────┐  ┌────────┐                   │
//! │  │ App A  │  │ App B  │  │Flutter │ ← future          │
//! │  └───┬────┘  └───┬────┘  └───┬────┘                    │
//! │      │ IPC        │ IPC       │ IPC                    │
//! │  ┌───▼────────────▼───────────▼────────────────────┐  │
//! │  │          Window Manager (THIS CRATE)            │  │
//! │  │  ┌──────┐ ┌───────────┐ ┌───────┐ ┌───────────┐  │  │
//! │  │  │window│ │compositor │ │cursor│ │input_router│  │  │
//! │  │  │.rs   │ │.rs        │ │.rs   │ │.rs         │  │  │
//! │  │  │Z-ord │ │blit/clip  │ │softw.│ │dispatch   │  │  │
//! │  │  └──────┘ └───────────┘ └───────┘ └───────────┘  │  │
//! │  │  ┌──────────────────────────────────────────────┐  │  │
//! │  │  │ flutter_api.rs (reserved surface interface) │  │  │
//! │  │  └──────────────────────────────────────────────┘  │  │
//! │  └────────────────────────────────────────────────────┘  │
//! │                  │ fb_commit  │ poll_input              │
//! ├──────────────────┼────────────┼──────────────────────────┤
//! │  AeroOS Kernel (Ring 0)                                  │
//! │  framebuffer | PS/2 keyboard | PS/2 mouse | time         │
//! └───────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Constraints
//!
//! - Pure software rendering — no GPU, no hardware acceleration
//! - Single mouse only — no multi-mouse support
//! - Flutter Engine must connect via the reserved surface API
//!
//! [MANUAL] All syscall wrappers return ENOSYS until Ring3 is
//! implemented. The `skeleton` feature provides fallback values.

#![no_std]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(mismatched_lifetime_syntaxes)]

pub mod compositor;
pub mod cursor;
pub mod flutter_api;
pub mod input_router;
pub mod syscalls;
pub mod window;

use compositor::Compositor;
use cursor::Cursor;
use input_router::InputRouter;
use window::WindowList;

/// Global WM state.
struct WmState {
    compositor: Compositor,
    cursor: Cursor,
    router: InputRouter,
    windows: WindowList,
    running: bool,
}

static mut WM: WmState = WmState {
    compositor: Compositor::new(),
    cursor: Cursor::new(),
    router: InputRouter::new(),
    windows: WindowList::new(),
    running: false,
};

/// Initialise the window manager.
///
/// 1. Query framebuffer geometry via `get_framebuffer_info`
/// 2. Initialise the compositor (off-screen buffer)
/// 3. Centre the cursor
/// 4. Create a test window (skeleton demonstration)
pub fn init() -> Result<(), i32> {
    let wm = unsafe { &mut WM };

    // 1. Initialise compositor (queries framebuffer info).
    wm.compositor.init()?;

    // 2. Centre cursor.
    wm.cursor.x = wm.compositor.width() as i32 / 2;
    wm.cursor.y = wm.compositor.height() as i32 / 2;

    // 3. Create a test window (skeleton — no real app connected).
    let test_w = 320u32;
    let test_h = 200u32;
    let cx = (wm.compositor.width() as i32 - test_w as i32) / 2;
    let cy = (wm.compositor.height() as i32 - test_h as i32) / 2;

    // [MANUAL] In Ring3, content_buf would be mmap'd shared memory.
    // For skeleton, pass null — compositor fills with dark background.
    wm.windows.create(
        cx,
        cy,
        test_w,
        test_h,
        b"AeroOS WM",
        core::ptr::null_mut(),
        0,
    );

    wm.running = true;
    Ok(())
}

/// Main WM event loop — never returns.
///
/// Each iteration:
/// 1. Poll for input events (keyboard/mouse)
/// 2. Route events to windows (focus, drag, etc.)
/// 3. Composite all windows to off-screen buffer
/// 4. Draw cursor on top
/// 5. Commit off-screen buffer to framebuffer
/// 6. Sleep until next tick
///
/// [MANUAL] When Ring3 is implemented, this runs as a user-mode
/// process. The `int 0x80` syscalls will actually work.
pub fn run() -> ! {
    let wm = unsafe { &mut WM };
    let fb_w = wm.compositor.width();
    let fb_h = wm.compositor.height();

    loop {
        let mut event = syscalls::InputEvent::default();

        // 1. Poll input (drain all pending events).
        loop {
            let ret = syscalls::poll_input(&mut event);
            if ret <= 0 {
                break;
            }
            let _target = wm.router.process_event(
                &event,
                &mut wm.cursor,
                &mut wm.windows,
                fb_w,
                fb_h,
            );
            // [MANUAL] Route the event to the target app process
            // via IPC (pipe, shared memory, or message queue).
        }

        // 2. Composite all windows.
        wm.compositor.composite(&wm.windows);

        // 3. Draw cursor on top.
        wm.cursor.draw(&wm.compositor);

        // 4. Commit to physical framebuffer.
        wm.compositor.commit();

        // 5. Sleep ~1ms (PIT tick) to avoid 100% CPU spin.
        syscalls::nanosleep(1);
    }
}
