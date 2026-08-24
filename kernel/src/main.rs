//! AeroOS kernel binary (standalone build).
//!
//! When built as a standalone `x86_64-unknown-none` binary, this crate
//! wires the kernel library's `kernel_main` to the bootloader entry point.
//! The `os` crate uses `bootloader::main_entry!` instead for image builds.

#![no_std]
#![no_main]

extern crate alloc;

use aeros_kernel::println;
use core::panic::PanicInfo;

bootloader_api::entry_point!(aeros_kernel::kernel_main);

/// Panic handler: dump the message over serial + VGA then halt.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[panic] {}", info);
    aeros_kernel::hlt_loop();
}
