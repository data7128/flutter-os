//! AeroOS kernel library — minimal x86_64 bare-metal kernel.
//!
//! Provides core kernel subsystems (VGA text, serial, GDT/IDT/PIC, heap
//! allocation) and a `kernel_main` entry point that initialises everything
//! and runs a keyboard-echo loop.

#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod interrupts;
pub mod mem;
pub mod memory;
pub mod serial;
pub mod vga_buffer;

use bootloader_api::config::{BootloaderConfig, Mapping};

/// Bootloader configuration: request mapping of all physical memory at a
/// dynamic offset. This makes the VGA text buffer (at physical 0xB8000)
/// accessible at `0xB8000 + physical_memory_offset`. The heap uses a
/// static array, so it doesn't depend on this mapping.
pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

/// Halt the CPU until the next interrupt, repeatedly.
pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

/// Kernel entry point called by the bootloader in 64-bit long mode.
pub fn kernel_main(boot_info: &'static mut bootloader_api::BootInfo) -> ! {
    // 0. Serial first — it doesn't depend on any memory mapping.
    serial::init();
    serial::_print(format_args!("[boot] AeroOS kernel starting\n"));

    // 0.5. VGA text buffer: needs the physical memory offset from the
    // bootloader to translate 0xB8000 to its virtual address.
    let phys_offset = boot_info
        .physical_memory_offset
        .into_option()
        .expect("[boot] bootloader did not map physical memory");
    vga_buffer::init(phys_offset);

    // Now println! writes to both serial and VGA.
    println!("[boot] AeroOS kernel starting");

    // 1. GDT → IDT → PIC
    interrupts::init();
    println!("[boot] interrupts online (GDT + IDT + 8259 PIC)");

    // 2. Heap allocator
    memory::init();
    println!("[boot] heap allocator online (1 MiB)");

    // 3. Enable keyboard IRQ
    interrupts::enable_keyboard();
    println!("[boot] PS/2 keyboard enabled");
    println!("[boot] AeroOS ready — type to echo to screen.\n");

    // 4. Main loop: halt until interrupt, then drain keyboard queue.
    loop {
        x86_64::instructions::hlt();
        while let Some(scancode) = interrupts::SCANCODE_BUFFER.lock().pop() {
            // Ignore key-release codes (bit 7 set) and E0/E1 prefixes.
            if scancode >= 0x80 || scancode == 0xe0 || scancode == 0xe1 {
                continue;
            }
            if let Some(ch) = scancode_to_ascii(scancode) {
                print!("{}", ch as char);
            } else {
                println!("\n[key] scan=0x{:02x}", scancode);
            }
        }
    }
}

/// Combine serial + VGA output so a single `println!` reaches both.
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

/// Map a PS/2 Set-1 make code to lowercase ASCII (printable keys only).
fn scancode_to_ascii(sc: u8) -> Option<u8> {
    Some(match sc {
        0x02 => b'1', 0x03 => b'2', 0x04 => b'3', 0x05 => b'4', 0x06 => b'5',
        0x07 => b'6', 0x08 => b'7', 0x09 => b'8', 0x0a => b'9', 0x0b => b'0',
        0x0c => b'-', 0x0d => b'=', 0x0e => 0x08, /* backspace */ 0x0f => b'\t',
        0x10 => b'q', 0x11 => b'w', 0x12 => b'e', 0x13 => b'r', 0x14 => b't',
        0x15 => b'y', 0x16 => b'u', 0x17 => b'i', 0x18 => b'o', 0x19 => b'p',
        0x1a => b'[', 0x1b => b']', 0x1c => b'\n',
        0x1e => b'a', 0x1f => b's', 0x20 => b'd', 0x21 => b'f', 0x22 => b'g',
        0x23 => b'h', 0x24 => b'j', 0x25 => b'k', 0x26 => b'l',
        0x27 => b';', 0x28 => b'\'', 0x29 => b'`', 0x2b => b'\\',
        0x2c => b'z', 0x2d => b'x', 0x2e => b'c', 0x2f => b'v', 0x30 => b'b',
        0x31 => b'n', 0x32 => b'm', 0x33 => b',', 0x34 => b'.', 0x35 => b'/',
        0x39 => b' ',
        _ => return None,
    })
}

/// QEMU ISA-debug-exit device codes (for integration tests).
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

/// `alloc` error handler: OOM is unrecoverable in the kernel.
#[alloc_error_handler]
pub fn alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("allocation error: {:?}", layout);
}
