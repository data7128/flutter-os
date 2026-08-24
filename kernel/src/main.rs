//! AeroOS kernel binary — freestanding `x86_64-unknown-none` entry point.
//!
//! Wires `kernel_main` to the bootloader's `entry_point!` macro with a
//! custom `BootloaderConfig` that requests physical-memory mapping.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

bootloader_api::entry_point!(aeros_kernel::kernel_main, config = &aeros_kernel::BOOTLOADER_CONFIG);

/// Panic handler: dump the message over serial + VGA, then halt.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    aeros_kernel::println!("[panic] {}", info);
    aeros_kernel::hlt_loop();
}
