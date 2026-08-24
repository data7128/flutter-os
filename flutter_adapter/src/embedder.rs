//! Flutter Engine embedder API skeleton.
//!
//! This module defines the **minimal subset** of the Flutter Engine
//! embedder C API that our adaptation layer calls. The real API is
//! defined in `flutter_embedder.h` (from the Flutter Engine repo).
//! We re-define compatible types here to avoid depending on a crate
//! that doesn't exist for `no_std`.
//!
//! ## What the embedder API does
//!
//! The Flutter Engine embedder API (sometimes called the "shell" API)
//! is a C-ABI interface that lets a host application:
//!
//! 1. Create and configure a Flutter Engine instance
//! 2. Push platform events (input, window resize, etc.)
//! 3. Receive rendering callbacks (the engine asks for a backing store,
//!    composites layers, and gives the host a finished frame)
//! 4. Drive the engine's task scheduler
//!
//! ## Integration points
//!
//! Each function below is marked with which part is AI-generated
//! skeleton vs which part needs manual implementation.
//!
//! [MANUAL] ALL functions here require the real `libflutter_engine.so`
//! to be loaded via ELF dynamic linker. This cannot happen until:
//! - Ring3 usermode is implemented
//! - ELF loader + dynamic linker is built
//! - libc compatibility layer exists
//! - pthread support is available

use crate::framebuffer::Color;
use crate::input::{InputEvent, PointerPhase};

// ── Flutter Engine C API types (re-defined from flutter_embedder.h) ───

/// Opaque handle to a running Flutter Engine instance.
///
/// [MANUAL] In the real API, this is `FlutterEngine` (a pointer).
pub type FlutterEngineHandle = *mut u8;

/// Engine result codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FlutterEngineResult {
    Success = 0,
    InvalidArgs = 1,
    OutOfMemory = 2,
    Failure = 3,
}

/// Renderer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FlutterRendererType {
    /// Software rasteriser — output is a raw pixel buffer.
    /// This is what we'll use initially (no GPU/EGL).
    Software = 0,
    /// OpenGL ES — requires EGL/GLES (not yet available).
    OpenGL = 1,
    /// Metal — Apple only.
    Metal = 2,
    /// Vulkan — not yet available.
    Vulkan = 3,
}

/// Software backing store — the engine renders into this buffer.
///
/// When `FlutterRendererType::Software` is used, the engine fills
/// this struct with a pointer to the rendered frame. The host
/// (our adapter) then blits it to the framebuffer.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FlutterSoftwareBackingStore {
    /// Pointer to the rendered pixel buffer.
    pub buffer: *mut u8,
    /// Length in bytes.
    pub buffer_size: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in bytes.
    pub stride: u32,
    /// Pixel format (0 = RGBA8888, 1 = BGRA8888).
    pub format: u32,
}

/// Engine configuration.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FlutterEngineConfig {
    /// Path to `icudtl.dat`.
    pub icu_data_path: *const u8,
    /// Path to Flutter assets directory.
    pub assets_path: *const u8,
    /// Path to AOT `app.so`.
    pub aot_library_path: *const u8,
    /// Renderer type (Software for our kernel).
    pub renderer_type: FlutterRendererType,
    /// Framebuffer width.
    pub width: u32,
    /// Framebuffer height.
    pub height: u32,
}

impl Default for FlutterEngineConfig {
    fn default() -> Self {
        Self {
            icu_data_path: b"/sys/icudtl.dat\0".as_ptr(),
            assets_path: b"/sys/flutter_assets/\0".as_ptr(),
            aot_library_path: b"/sys/app.so\0".as_ptr(),
            renderer_type: FlutterRendererType::Software,
            width: 1280,
            height: 720,
        }
    }
}

// ── Embedder API functions (skeleton) ─────────────────────────────────

/// Initialise the Flutter Engine.
///
/// [MANUAL] This calls the real `FlutterEngineInitialize()` from
/// `libflutter_engine.so`. Requires:
/// - ELF dynamic linker to load `libflutter_engine.so`
/// - libc (malloc, pthread, etc.)
/// - `icudtl.dat` and `app.so` on the FAT32 filesystem
/// - Software rasteriser (Skia) compiled into the engine
pub fn init() -> Result<(), i32> {
    // [MANUAL] Real implementation:
    //   let engine = FlutterEngineInitialize(&config, &callbacks, ...);
    //   if engine != nullptr {
    //       FlutterEngineRun(engine);
    //   }

    // SKELETON: log that we're not ready.
    // crate::syscalls::write(1, b"[flutter] embedder init: ENOSYS (no engine loaded)\n");
    Err(crate::syscalls::ENOSYS as i32)
}

/// Send a pointer (mouse/touch) event to the engine.
///
/// [MANUAL] Real implementation calls `FlutterEngineSendPointerEvent()`.
pub fn send_pointer_event(_x: f64, _y: f64, _phase: PointerPhase) {
    // [MANUAL] FlutterEngineSendPointerEvent(engine, &event, 1);
}

/// Send a keyboard event to the engine.
///
/// [MANUAL] Real implementation calls `FlutterEngineSendKeyEvent()`
/// (added in newer Flutter Engine versions).
pub fn send_key_event(_keycode: u32, _keychar: u32, _phase: u8) {
    // [MANUAL] FlutterEngineSendKeyEvent(engine, &event, 1);
}

/// Dispatch a frame render.
///
/// Called from the main loop. The engine will:
/// 1. Ask for a backing store (we provide a software buffer)
/// 2. Rasterise the Dart UI into that buffer
/// 3. Return the finished frame
/// 4. We blit the buffer to the framebuffer
///
/// [MANUAL] This is the core rendering loop. The real implementation
/// involves `FlutterEngineOnVsync()` → engine calls
/// `FlutterOnAcquireBackingStore()` → engine renders →
/// `FlutterOnPresentBackingStore()`.
pub fn dispatch_frame() {
    // [MANUAL] Real flow:
    //   1. FlutterEngineOnVsync(engine, ...)
    //   2. Engine calls on_acquire_backing_store → we provide fb buffer
    //   3. Engine renders Skia/Impeller into the buffer
    //   4. Engine calls on_present_backing_store → we blit to screen
    //   5. Swap buffers
}

/// Shut down the engine.
///
/// [MANUAL] Calls `FlutterEngineShutdown()`.
pub fn shutdown() {
    // [MANUAL] FlutterEngineShutdown(engine);
}

// ── Software rendering path (skeleton) ────────────────────────────────

/// The "software rasteriser" callback: when the engine asks for a
/// backing store, we provide a buffer for it to render into.
///
/// [MANUAL] In the real API, this is a C callback registered with
/// `FlutterRendererConfig`. The engine calls it before rendering
/// each frame.
pub fn on_acquire_backing_store(
    backing_store: &mut FlutterSoftwareBackingStore,
) -> FlutterEngineResult {
    // [MANUAL] We allocate a buffer the same size as the framebuffer.
    // The engine renders into it, then we blit to the actual fb.
    //
    // For now, this is a skeleton — returns Failure.
    let _ = backing_store;
    FlutterEngineResult::Failure
}

/// The "present" callback: after the engine renders a frame into our
/// backing store, this function blits it to the real framebuffer.
///
/// [MANUAL] This is where we call `framebuffer::canvas().blit(...)`.
pub fn on_present_backing_store(
    backing_store: &FlutterSoftwareBackingStore,
) -> FlutterEngineResult {
    // [MANUAL] Real implementation:
    //   crate::framebuffer::canvas().blit(
    //       core::slice::from_raw_parts(backing_store.buffer,
    //           backing_store.buffer_size as usize),
    //       backing_store.width,
    //       backing_store.height,
    //       0, 0,
    //   );

    let _ = backing_store;
    FlutterEngineResult::Failure
}
