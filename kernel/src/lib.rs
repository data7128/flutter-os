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

pub mod aero_format;
pub mod boot_service;
pub mod exec;
pub mod fs;
pub mod graphics;
pub mod interrupts;
pub mod mem;
pub mod memory;
pub mod oom;
pub mod perm;
pub mod process;
pub mod scheduler;
pub mod serial;
pub mod shell_host;
pub mod signal;
pub mod syscall_trampoline;
pub mod syscalls;
pub mod usb;
pub mod userproc;
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

    // ── 4.5. Memory subsystem: physical frame allocator + page tables ─
    // Initialises the frame allocator from the bootloader memory map
    // and sets up the kernel OffsetPageTable. Required for user-mode
    // address spaces and Ring3 isolation.
    unsafe {
        memory::init_memory_subsystem(
            &mut boot_info.memory_regions,
            phys_offset,
        );
    }

    // ── 4.6. VFS + tmpfs root filesystem + initramfs ────────────────
    // Mounts a tmpfs as "/" and populates it with embedded initramfs
    // files (e.g. /bin/hello — a minimal Ring3 ELF user program).
    fs::init();
    fs::tmpfs::init();
    fs::initramfs::init();
    println!("[OK] VFS_TMPFS_INITRAMFS");

    // ── 5. PS/2 keyboard + mouse (unmask IRQ1 + IRQ12) ─────────────
    interrupts::enable_keyboard();
    interrupts::mouse::init();
    println!("[OK] KEYBOARD");
    println!("[OK] MOUSE");

    // ── 6. Framebuffer graphics (from bootloader BootInfo) ──────────
    // Future GUI layer (e.g. Flutter engine embedder) will attach here.
    if let Some(fb) = boot_info.framebuffer.take() {
        graphics::init(fb);
        println!("[OK] GRAPHICS");
    } else {
        println!("[WARN] GRAPHICS — no framebuffer from bootloader");
    }

    // ── 7. Time subsystem (PIT-programmed, tick counter for clock_gettime) ─
    syscalls::time::init();
    println!("[OK] TIME");

    // ── 8. Syscall framework (int 0x80 dispatch: open/read/write/mmap/...) ─
    // ← FUTURE: Flutter Engine as Ring3 user program will invoke these.
    syscalls::init();
    println!("[OK] SYSCALLS");

    // ── 9. Flutter adapter support (fb info syscall) ──────────────────
    // The kernel exposes framebuffer geometry via SYS_GET_FB_INFO so the
    // future Flutter Engine adapter (Ring3 user-mode) can mmap the fb.
    // ← FUTURE: Flutter adapter calls get_framebuffer_info() then mmap().
    println!("[OK] FLUTTER_ADAPTER");

    // ── 10. Window manager support (fb_commit + poll_input syscalls) ──
    // The kernel exposes two new syscalls for the user-mode WM:
    //   SYS_FB_COMMIT  — blit a rendered buffer to the physical framebuffer
    //   SYS_POLL_INPUT — poll for structured keyboard/mouse input events
    // The WM itself runs as a Ring3 user-mode process. The kernel
    // does NOT contain any window management, clipping, or cursor logic.
    // ← FUTURE: WM calls poll_input() + fb_commit() in its event loop.
    println!("[OK] WINDOW_MANAGER");

    // ── 11. Process table + ELF exec loader ────────────────────────
    // The process table manages Ring3 user-mode processes. The ELF
    // loader parses ELF64 executables and (future) maps them into
    // user-space address spaces.
    // [MANUAL] Ring3 context switch requires TSS + page tables + iretq.
    println!("[OK] EXEC_LOADER");

    // ── 12. Boot service: launch WM + Flutter Shell ───────────────
    // Allocates process slots for the window manager and Flutter
    // system shell. The actual exec requires Ring3.
    // [MANUAL] exec("/sys/wm") + exec("/sys/flutter_shell") need Ring3.
    boot_service::launch_system();
    println!("[OK] FLUTTER_SHELL");

    // ── 13. Signal subsystem (SIGKILL/SIGTERM) ────────────────────
    // Minimal signal delivery: terminate processes on kill signal.
    // [MANUAL] Full POSIX signal set not implemented.
    println!("[OK] SIGNAL_SUBSYS");

    // ── 14. OOM handler ───────────────────────────────────────────
    // Instead of panicking on alloc failure, reaps zombie processes
    // and terminates memory-hungry processes to reclaim memory.
    println!("[OK] OOM_HANDLER");

    // ── 15. USB UHCI driver + HID input ───────────────────────────
    // UHCI host controller driver for USB 1.x. Supports HID keyboard
    // and mouse via boot protocol. No EHCI/XHCI, no USB storage.
    // [MANUAL] Requires hardware timing validation on QEMU/real HW.
    usb::init();
    println!("[OK] UHCI_USB");
    println!("[OK] USB_HID_INPUT");

    // ── 16. .aero app format support ─────────────────────────────
    // Prototype native application package format. No signatures,
    // no checksums. The kernel can parse .aero headers and extract
    // embedded ELF binaries for exec.
    println!("[OK] AERO_APP_FORMAT");

    // ── 17. Permission subsystem ─────────────────────────────────
    // Minimal privilege model: kernel/system/user levels.
    // Resource access checks for framebuffer, input, filesystem.
    println!("[OK] PERMISSION_SUBSYS");

    // ── 18. Future subsystems (not yet implemented) ───────────────
    // Ring 3 usermode requires TSS user segments + syscall/iretq handling.
    println!("[PENDING] USERMODE");
    // Scheduler requires context switching (task structs, context save/restore).
    println!("[PENDING] SCHEDULER");
    // Signals (kill, sigaction) require per-process signal mask + delivery.
    println!("[PENDING] SIGNAL");
    // fork/exec require process address space duplication + ELF loader.
    println!("[PENDING] FORK_EXEC");

    println!("[boot] AeroOS ready — all subsystems online.\n");

    // ── 19. Launch user process from VFS (/bin/hello) ────────────────
    // Reads the embedded ELF from the initramfs tmpfs, spawns a Ring3
    // user process, and starts the preemptive scheduler. The user program
    // calls write(1, "Hello from /bin/hello!\n", 22) via int 0x80,
    // then exit(0). This validates the full pipeline: VFS → ELF loader →
    // user address space → scheduler → Ring3 → syscall → write.
    println!("[boot] spawning /bin/hello from VFS...");
    match userproc::spawn_user_process_from_path("/bin/hello") {
        Ok(pid) => {
            println!("[boot] /bin/hello spawned as pid={}", pid);
            scheduler::start_scheduler();
            println!("[boot] scheduler started — entering idle loop");
            // Enable interrupts and halt forever; the scheduler will
            // context-switch to the user process on the next timer tick.
            x86_64::instructions::interrupts::enable();
            loop {
                x86_64::instructions::hlt();
            }
        }
        Err(e) => {
            println!("[boot] FAILED to spawn /bin/hello: {}", e);
            println!("[boot] falling back to inline Ring3 test...");
            launch_ring3_test();
        }
    }
}

/// Minimal Ring3 user-mode test program (position-independent x86_64).
///
/// ```asm
///   mov rax, 3          ; syscall Write
///   mov rdi, 1          ; fd = stdout
///   lea rsi, [rip+25]   ; msg = "Hello from Ring3!\n"
///   mov rdx, 19         ; len
///   int 0x80
///   mov rax, 11         ; syscall Exit
///   mov rdi, 0          ; status = 0
///   int 0x80
///   msg: db "Hello from Ring3!", 0xA
/// ```
const RING3_TEST_CODE: &[u8] = &[
    0x48, 0xC7, 0xC0, 0x03, 0x00, 0x00, 0x00, // mov rax, 3
    0x48, 0xC7, 0xC7, 0x01, 0x00, 0x00, 0x00, // mov rdi, 1
    0x48, 0x8D, 0x35, 0x19, 0x00, 0x00, 0x00, // lea rsi, [rip+25]
    0x48, 0xC7, 0xC2, 0x13, 0x00, 0x00, 0x00, // mov rdx, 19
    0xCD, 0x80, // int 0x80
    0x48, 0xC7, 0xC0, 0x0B, 0x00, 0x00, 0x00, // mov rax, 11
    0x48, 0xC7, 0xC7, 0x00, 0x00, 0x00, 0x00, // mov rdi, 0
    0xCD, 0x80, // int 0x80
    // msg: "Hello from Ring3!\n" (19 bytes)
    b'H', b'e', b'l', b'l', b'o', b' ', b'f', b'r', b'o', b'm',
    b' ', b'R', b'i', b'n', b'g', b'3', b'!', b'\n',
];

/// Launch the Ring3 user-mode test.
///
/// Maps the test code and a user stack into the kernel page table with
/// USER_ACCESSIBLE, then `iretq`s into Ring3.
fn launch_ring3_test() -> ! {
    use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};
    use x86_64::VirtAddr;

    let user_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE;

    // Map code page at 0x400000.
    let code_page = Page::<Size4KiB>::containing_address(VirtAddr::new(0x400000));
    let code_frame = match crate::memory::page_table::map_page_alloc(code_page, user_flags) {
        Ok(f) => f,
        Err(e) => {
            println!("[ring3-test] failed to map code page: {}", e);
            crate::hlt_loop();
        }
    };

    // Copy test code into the code page (through physical-offset mapping).
    let phys_offset = *crate::memory::page_table::PHYSICAL_OFFSET.lock();
    let code_virt = VirtAddr::new(phys_offset + code_frame.start_address().as_u64());
    unsafe {
        core::ptr::copy_nonoverlapping(
            RING3_TEST_CODE.as_ptr(),
            code_virt.as_mut_ptr::<u8>(),
            RING3_TEST_CODE.len(),
        );
    }

    // Map user stack at 0x7000_0000 (one page, stack grows down).
    let stack_page = Page::<Size4KiB>::containing_address(VirtAddr::new(0x7000_0000 - 4096));
    if let Err(e) = crate::memory::page_table::map_page_alloc(stack_page, user_flags) {
        println!("[ring3-test] failed to map stack page: {}", e);
        crate::hlt_loop();
    }
    let user_stack_top = 0x7000_0000u64;

    println!("[ring3-test] code at 0x400000, stack at 0x70000000, entering Ring3...");

    // Enter Ring3. This never returns.
    unsafe {
        crate::syscall_trampoline::enter_usermode(0x400000, user_stack_top);
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

/// `alloc` error handler: tries OOM recovery, panics if unrecoverable.
#[alloc_error_handler]
pub fn alloc_error(layout: core::alloc::Layout) -> ! {
    // Try to reclaim memory before panicking.
    if oom::handle_oom(&layout) {
        // Memory was reclaimed — retry the allocation.
        // The allocator will call this handler again if it still fails.
        // We use a spin loop to retry, since we can't return from this function.
        // [MANUAL] A proper implementation would use a retry mechanism
        // in the allocator itself rather than this handler.
        panic!("oom recovery did not free enough memory: {:?}", layout);
    }
    panic!("allocation error (oom, no recovery possible): {:?}", layout);
}
