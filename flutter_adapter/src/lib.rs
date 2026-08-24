//! Flutter Engine user-mode adaptation layer for AeroOS.
//!
//! This crate is the **skeleton** of the adaptation layer that will sit
//! between the Flutter Engine and our custom kernel. It is designed to
//! run as a **Ring 3 user-mode program** — never in kernel space.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │            Flutter Engine (libflutter_engine.so)
//! │    (Dart runtime + Skia/Impeller + compositor)
//! └──────────────────┬──────────────────────────┘
//!                     │  Flutter Embedder API
//! │                     │  (FlutterEngineRun, etc.)
//! │ ┌──────────────────▼──────────────────────────┐
//! │ │         flutter_adapter (THIS CRATE)        │
//! │ │  ┌─────────┐ ┌────────────┐ ┌───────────┐  │
//! │ │  │ input.rs│ │framebuffer │ │ embedder  │  │
//! │ │  │         │ │    .rs     │ │    .rs    │  │
//! │ │  │PS/2→    │ │mmap(fb)→   │ │Engine API │  │
//! │ │  │Flutter  │ │Canvas      │ │skeleton   │  │
//! │ │  │events   │ │            │ │           │  │
//! │ │  └────┬────┘ └─────┬──────┘ └─────┬─────┘  │
//! │ │       │             │              │        │
//! │ │  ┌────▼─────────────▼──────────────▼─────┐  │
//! │ │  │           syscalls.rs                 │  │
//! │ │  │   int 0x80 → kernel (read/write/mmap) │  │
//! │ │  └───────────────────────────────────────┘  │
//! │ └──────────────────────────────────────────────┘
//! │                     │
//! ────────────── AeroOS kernel (Ring 0) ──────────
//! │  GDT/IDT/PIC | framebuffer | PS/2 | heap | time │
//! └─────────────────────────────────────────────────
//! ```
//!
//! ## Current status: SKELETON ONLY
//!
//! All code here is **generated skeleton** — it compiles but does NOT
//! actually run Flutter Engine. The following sections need manual work:
//!
//! - [MANUAL] Ring3 user-mode entry point (needs kernel TSS + iretq)
//! - [MANUAL] ELF loader to load libflutter_engine.so
//! - [MANUAL] Flutter Embedder API C FFI bindings
//! - [MANUAL] Skia/Impeller rendering backend
//! - [MANUAL] Dart isolate thread support

#![no_std]
#![allow(dead_code)]
#![allow(static_mut_refs)]

pub mod embedder;
pub mod framebuffer;
pub mod input;
pub mod syscalls;

/// Initialise the adaptation layer.
///
/// Call order:
/// 1. `syscalls::init()` — verify syscall interface is available
/// 2. `framebuffer::init()` — mmap the framebuffer, create canvas
/// 3. `input::init()` — start keyboard event polling
/// 4. `embedder::init()` — initialise Flutter Engine (skeleton)
///
/// Returns `Ok(())` on success, `Err(code)` on failure.
pub fn init() -> Result<(), i32> {
    // 1. Verify syscall interface.
    syscalls::init()?;

    // 2. Map the framebuffer via mmap syscall.
    framebuffer::init()?;

    // 3. Start input event polling.
    input::init()?;

    // 4. Initialise Flutter Engine embedder (skeleton — returns ENOSYS).
    // [MANUAL] This requires the real libflutter_engine.so to be loaded.
    embedder::init()?;

    Ok(())
}

/// Main event loop — never returns.
///
/// Pseudo-code flow:
/// 1. Poll keyboard events from stdin (syscall read)
/// 2. Convert to Flutter input events
/// 3. Push to Flutter Engine via embedder API
/// 4. Engine renders frame → write to framebuffer canvas
/// 5. Sleep until next timer tick
///
/// [MANUAL] This loop needs the real Flutter Engine API calls.
pub fn run() -> ! {
    loop {
        // 1. Poll input
        let events = input::poll();

        // 2. Feed to engine (skeleton — no-op)
        for _event in events.iter() {
            // [MANUAL] embedder::send_pointer_event(event);
            // [MANUAL] embedder::send_key_event(event);
        }

        // 3. Engine renders (skeleton — we just clear the framebuffer)
        // [MANUAL] embedder::dispatch_frame();

        // 4. Sleep until next tick (1ms)
        syscalls::nanosleep(1);
    }
}
