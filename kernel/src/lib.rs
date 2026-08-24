//! AeroOS kernel library — minimal x86_64 bare-metal kernel.
//!
//! Provides core kernel subsystems (VGA text, serial, GDT/IDT/PIC, heap
//! allocation, framebuffer graphics) and a `kernel_main` entry point that
//! initialises everything and runs a keyboard-echo loop on the framebuffer.
//!
//! ## Subsystem boot markers (for CI)
//!
//! After each subsystem initialises, a `[OK] <NAME>` marker is printed to
//! the COM1 serial port. The CI script `ci/check_boot.sh` greps these
//! markers to determine if the kernel booted successfully.

#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod graphics;
pub mod interrupts;
pub mod mem;
pub mod memory;
pub mod serial;
pub mod shell_host;
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
///
/// Initialisation order: serial → VGA → GDT → IDT → PIC → heap →
/// keyboard → graphics → shell host. Each step prints a `[OK]` or
/// `[PENDING]` marker that the CI boot-test script parses.
pub fn kernel_main(boot_info: &'static mut bootloader_api::BootInfo) -> ! {
    // ── 0. Serial console (no memory mapping needed) ──────────────
    serial::init();
    serial::_print(format_args!("[boot] AeroOS kernel starting\n"));

    // ── 0.5. VGA text buffer (needs physical_memory_offset) ─────────
    let phys_offset = boot_info
        .physical_memory_offset
        .into_option()
        .expect("[boot] bootloader did not map physical memory");
    vga_buffer::init(phys_offset);

    // Now println! writes to both serial and VGA.
    println!("[boot] AeroOS kernel starting");

    // ── 1. GDT (code + data segments + TSS) ────────────────────────
    interrupts::gdt::init();
    println!("[OK] GDT");

    // ── 2. IDT (breakpoint, double fault, timer, keyboard) ─────────
    interrupts::idt::init();
    println!("[OK] IDT");

    // ── 3. 8259A PIC (remap + init + enable interrupts) ─────────────
    unsafe {
        interrupts::PICS.lock().initialize();
    }
    x86_64::instructions::interrupts::enable();
    println!("[OK] PIC");

    // ── 4. Heap allocator (1 MiB static BSS array) ──────────────────
    memory::init();
    println!("[OK] HEAP");

    // ── 5. PS/2 keyboard (unmask IRQ1) ──────────────────────────────
    interrupts::enable_keyboard();
    println!("[OK] KEYBOARD");

    // ── 6. Framebuffer graphics (from bootloader BootInfo) ──────────
    // Future GUI layer (e.g. Flutter engine embedder) will attach here.
    if let Some(fb) = boot_info.framebuffer.take() {
        graphics::init(fb);
        println!("[OK] GRAPHICS");
    } else {
        println!("[WARN] GRAPHICS — no framebuffer from bootloader");
    }

    // ── 7. Future subsystems (not yet implemented) ─────────────────
    // Ring 3 usermode requires TSS user segments + syscall/iretq handling.
    println!("[PENDING] USERMODE");
    // Scheduler requires context switching (task structs, context save/restore).
    println!("[PENDING] SCHEDULER");

    println!("[boot] AeroOS ready — all subsystems online.\n");

    // ── 8. Launch framebuffer shell host (never returns) ────────────
    // ← FUTURE: Flutter engine embedder would take over here, rendering
    //   Dart UI onto the same framebuffer.
    shell_host::launch();
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
pub fn scancode_to_ascii(sc: u8) -> Option<u8> {
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
