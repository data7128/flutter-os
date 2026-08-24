//! AeroOS kernel library.
//!
//! Re-exports every kernel subsystem so that the binary crate (`main.rs`)
//! and integration tests can use them through a single dependency.

#![no_std]
#![cfg_attr(test, allow(dead_code))]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod graphics;
pub mod interrupts;
pub mod memory;
pub mod serial;
pub mod shell_host;
pub mod vga_buffer;

use core::panic::PanicInfo;

/// Halt the CPU until the next interrupt, repeatedly. Safe idle loop.
pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

/// The kernel entry point. The `bootloader` crate jumps here in 64-bit
/// long mode with paging already enabled.
pub fn kernel_main(boot_info: &'static mut bootloader_api::BootInfo) -> ! {
    serial::init();
    println!("[boot] AeroOS kernel starting");

    // 1. GDT (segments) -> IDT (exceptions) -> PIC (hardware IRQs)
    interrupts::init();
    println!("[boot] interrupts online");

    // 2. Memory: seed the heap allocator from a usable region
    memory::init(boot_info);
    println!("[boot] heap allocator online");

    // 3. Linear framebuffer (the surface the Flutter shell renders to)
    graphics::init(boot_info);
    println!("[boot] framebuffer ready");

    // 4. Keyboard input -> host event queue, then run the shell
    interrupts::enable_keyboard();
    println!("[boot] keyboard input enabled");

    // 5. Launch the shell host (renders the desktop on the framebuffer).
    //    `launch` is `-> !` and never returns.
    shell_host::launch()
}

/// Combines serial + VGA so a single `println!` reaches both the serial
/// console (host logs) and the on-screen boot log.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        $crate::serial::_print(format_args!($($arg)*));
        $crate::vga_buffer::_print(format_args!($($arg)*));
    });
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

/// Test runner used by `cargo test --target x86_64-unknown-none`.
pub fn test_runner(tests: &[&dyn Fn()]) {
    serial::_print(format_args!("Running {} tests\n", tests.len()));
    for test in tests {
        test();
    }
    serial::_print(format_args!("\n[ok] tests done, exiting via QEMU\n"));
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial::_print(format_args!("[test-panic] {}\n", info));
    exit_qemu(QemuExitCode::Failed);
    loop {}
}

/// QEMU ISA-debug-exit device codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

/// `alloc` error handler: there is no recovery on OOM in the kernel,
/// so we log and halt.
#[alloc_error_handler]
pub fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("allocation error: {:?}", layout);
}
