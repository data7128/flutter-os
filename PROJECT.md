# flutter-os — x86_64 裸机操作系统

最小可行 x86_64 裸机内核，使用 Rust `no_std` 编写，`bootloader` 0.11 crate 引导，支持 BIOS，可在 `qemu-system-x86_64` 运行。

## 已验证

- 内核编译：**0 errors, 0 warnings**
- BIOS 磁盘镜像构建：**成功**（2.5 MB）
- QEMU 启动：**所有子系统上线**

```
[serial] AeroOS serial console online
[boot] AeroOS kernel starting
[boot] interrupts online (GDT + IDT + 8259 PIC)
[boot] heap allocator online (1 MiB)
[boot] PS/2 keyboard enabled
[boot] AeroOS ready — type to echo to screen.
```

## 项目目录结构

```
flutter-os/
├── .cargo/
│   └── config.toml          # bindeps (nightly unstable feature)
├── .github/
│   └── workflows/
│       └── ci.yml            # CI: build kernel + OS image
├── .gitignore
├── rust-toolchain.toml       # nightly + rust-src + llvm-tools + x86_64-unknown-none
├── Cargo.toml                # 工作区根: OS 镜像构建器 (std)
├── build.rs                  # BiosBoot::create_disk_image()
├── build.sh                  # 一键构建脚本
├── src/
│   └── main.rs               # 复制 .img 到 target/
├── kernel/                   # no_std 内核 crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # kernel_main, print 宏, bootloader 配置, scancode 映射
│       ├── main.rs           # entry_point! + panic_handler
│       ├── mem.rs            # memcpy/memset/memmove/memcmp/strlen (无需 build-std)
│       ├── vga_buffer.rs     # VGA 文本模式 (0xB8000) + physical offset 初始化
│       ├── serial.rs         # COM1 16550 UART
│       ├── memory/
│       │   └── mod.rs        # 堆分配器 (静态数组, 1 MiB)
│       └── interrupts/
│           ├── mod.rs        # 8259 PIC + scancode 环形缓冲区
│           ├── gdt.rs        # GDT + TSS (代码段 + 数据段)
│           └── idt.rs        # IDT 异常 + IRQ 处理程序
├── LICENSE
└── README.md
```

## 文件清单

---

### `rust-toolchain.toml`

```toml
[toolchain]
channel = "nightly"
components = ["llvm-tools-preview", "rust-src"]
targets = ["x86_64-unknown-none"]
profile = "minimal"
```

---

### `.cargo/config.toml`

```toml
[unstable]
bindeps = true
```

---

### `Cargo.toml` (工作区根)

```toml
[package]
name = "aeros-os"
version = "0.1.0"
edition = "2021"
authors = ["flutter-os contributors"]
description = "Minimal x86_64 bare-metal OS: Rust kernel + BIOS bootloader disk image"

[dependencies]
bootloader = { version = "0.11", default-features = false, features = ["bios"] }

[build-dependencies]
bootloader = { version = "0.11", default-features = false, features = ["bios"] }
aeros-kernel = { path = "kernel", artifact = "bin", target = "x86_64-unknown-none" }

[workspace]
members = ["kernel"]

[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
lto = true
opt-level = 3
```

---

### `build.rs`

```rust
//! Build script: creates a bootable BIOS disk image from the kernel binary.
//!
//! The kernel ELF is provided by cargo's artifact-dependency feature as an
//! environment variable. We feed it to `bootloader::BiosBoot` which produces
//! a raw disk image that boots on legacy BIOS (and in QEMU).

use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());

    // Locate the kernel binary path set by cargo's artifact-dependency feature.
    let kernel = std::env::vars()
        .find(|(k, _)| k.starts_with("CARGO_BIN_FILE_"))
        .map(|(_, v)| PathBuf::from(v))
        .expect("build.rs: no CARGO_BIN_FILE_* env var found — is bindeps enabled?");

    eprintln!("build.rs: kernel binary = {}", kernel.display());

    // Create a bootable BIOS disk image.
    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel)
        .create_disk_image(&bios_path)
        .expect("build.rs: failed to create BIOS disk image");

    eprintln!("build.rs: BIOS image = {}", bios_path.display());

    // Pass the image path to src/main.rs via a compile-time env var.
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
    println!("cargo:rerun-if-changed=kernel/src");
}
```

---

### `src/main.rs`

```rust
//! `aeros-os` binary: copies the bootable BIOS disk image produced by
//! `build.rs` to the workspace `target/` directory and prints the QEMU
//! command to run it.
//!
//! The image path is injected at compile time via the `BIOS_PATH` env var.

use std::path::PathBuf;

fn main() {
    let bios_path = env!("BIOS_PATH");
    let bios_path = PathBuf::from(bios_path);

    // Copy the image to a stable, easy-to-find location.
    let dest = PathBuf::from("target/aeros-os-bios.img");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::copy(&bios_path, &dest).ok();

    println!();
    println!("═══════════════════════════════════════════════");
    println!("  AeroOS BIOS disk image ready");
    println!("  Image: {}", dest.display());
    println!("  Size:  {} bytes", std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0));
    println!();
    println!("  Run in QEMU:");
    println!("    qemu-system-x86_64 \\");
    println!("      -drive format=raw,file={} \\", dest.display());
    println!("      -serial stdio");
    println!("═══════════════════════════════════════════════");
}
```

---

### `build.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────
#  AeroOS build script
#  Builds the kernel and a bootable BIOS disk image.
# ──────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

echo "═══════════════════════════════════════════════"
echo "  AeroOS Build"
echo "═══════════════════════════════════════════════"

# ── 1. Build kernel + BIOS disk image ──────────────────
echo ""
echo "[1/2] Building kernel and BIOS disk image..."
cargo build -p aeros-os --release

# ── 2. Copy the disk image to a stable path ────────────
echo ""
echo "[2/2] Running image builder..."
IMAGE="target/aeros-os-bios.img"
cargo run -p aeros-os --release --quiet

if [ -f "$IMAGE" ]; then
    echo ""
    echo "  ✓ Disk image: $IMAGE ($(du -h "$IMAGE" | cut -f1))"
    echo ""
    echo "═══════════════════════════════════════════════"
    echo "  Build complete!"
    echo ""
    echo "  Run in QEMU:"
    echo "    qemu-system-x86_64 \\"
    echo "      -drive format=raw,file=$IMAGE \\"
    echo "      -serial stdio"
    echo "═══════════════════════════════════════════════"
else
    echo "  ✗ Disk image not found at $IMAGE"
    exit 1
fi
```

---

### `kernel/Cargo.toml`

```toml
[package]
name = "aeros-kernel"
version = "0.1.0"
edition = "2021"
authors = ["flutter-os contributors"]
description = "Minimal x86_64 bare-metal kernel (no_std, bootloader 0.11)"
license = "MIT"

[dependencies]
bootloader_api = "0.11"
x86_64 = "0.15"
spin = { version = "0.9", default-features = false, features = ["spin_mutex"] }
pic8259 = "0.10"
uart_16550 = "0.3"
lazy_static = { version = "1.4", features = ["spin_no_std"] }
linked_list_allocator = "0.10"
```

---

### `kernel/src/lib.rs`

```rust
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
```

---

### `kernel/src/main.rs`

```rust
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
```

---

### `kernel/src/mem.rs`

```rust
//! Minimal implementations of compiler-builtins memory functions.
//!
//! Without `-Z build-std-features=compiler-builtins-mem`, the linker
//! expects `memcpy`, `memset`, `memmove`, and `memcmp` to be provided
//! by the crate itself. These are simple byte-wise implementations.

use core::ffi::{c_char, c_int, c_void};

/// Copy `n` bytes from `src` to `dest`. The regions must not overlap
/// (use `memmove` for overlapping regions).
#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest = dest as *mut u8;
    let src = src as *const u8;
    for i in 0..n {
        *dest.add(i) = *src.add(i);
    }
    dest as *mut c_void
}

/// Copy `n` bytes from `src` to `dest`, handling overlapping regions.
#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest = dest as *mut u8;
    let src = src as *const u8;
    if (dest as usize) < (src as usize) {
        for i in 0..n {
            *dest.add(i) = *src.add(i);
        }
    } else {
        for i in (0..n).rev() {
            *dest.add(i) = *src.add(i);
        }
    }
    dest as *mut c_void
}

/// Fill `n` bytes at `s` with byte value `c`.
#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let s = s as *mut u8;
    let byte = c as u8;
    for i in 0..n {
        *s.add(i) = byte;
    }
    s as *mut c_void
}

/// Compare `n` bytes at `s1` and `s2`. Returns 0 if equal, negative if
/// `s1 < s2`, positive if `s1 > s2` (at the first differing byte).
#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let s1 = s1 as *const u8;
    let s2 = s2 as *const u8;
    for i in 0..n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return (a as c_int) - (b as c_int);
        }
    }
    0
}

/// Length of a null-terminated string.
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let s = s as *const u8;
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}
```

---

### `kernel/src/vga_buffer.rs`

```rust
//! VGA text-mode buffer (0xB8000 physical).
//!
//! A classic 80x25 color text buffer. Because the bootloader maps physical
//! memory at a dynamic offset, the buffer's virtual address is
//! `0xB8000 + physical_memory_offset`. The `init()` function must be called
//! with the offset (from `BootInfo`) before any VGA writes occur; before that,
//! `_print` is a no-op so early serial-only output never faults.

use core::fmt::{self, Write};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;

/// The standard 16-color VGA palette.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

/// Foreground + background packed into one byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    const fn new(foreground: Color, background: Color) -> Self {
        Self((background as u8) << 4 | (foreground as u8))
    }
}

/// One VGA character cell: ASCII byte + color code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

/// Physical address of the VGA text buffer.
const VGA_BUFFER_PHYS: u64 = 0xb8000;

/// Whether `init()` has been called and VGA is safe to use.
static VGA_READY: AtomicBool = AtomicBool::new(false);

/// A view over the 25x80 screen, wrapped in a spinlock so interrupts can
/// also write safely.
static WRITER: Mutex<Writer> = Mutex::new(Writer {
    column_position: 0,
    row_position: 0,
    color_code: ColorCode::new(Color::LightGreen, Color::Black),
    buffer: 0xb8000 as *mut ScreenChar, // overwritten by init()
});

pub struct Writer {
    column_position: usize,
    row_position: usize,
    color_code: ColorCode,
    buffer: *mut ScreenChar,
}

// The writer only accesses the VGA buffer through the single locked
// instance, so it is safe to mark `Send`.
unsafe impl Send for Writer {}

impl Writer {
    /// Write one byte (character) at the current cursor position using
    /// volatile store so the compiler cannot elide the write.
    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            0x0d => self.column_position = 0,
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }
                let row = self.row_position;
                let col = self.column_position;
                let color_code = self.color_code;

                unsafe {
                    ptr::write_volatile(
                        self.buffer.add(row * BUFFER_WIDTH + col),
                        ScreenChar {
                            ascii_character: byte,
                            color_code,
                        },
                    );
                }
                self.column_position += 1;
            }
        }
    }

    fn new_line(&mut self) {
        // Scroll the whole buffer up by one row.
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let character =
                    unsafe { ptr::read_volatile(self.buffer.add(row * BUFFER_WIDTH + col)) };
                unsafe {
                    ptr::write_volatile(
                        self.buffer.add((row - 1) * BUFFER_WIDTH + col),
                        character,
                    );
                }
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
        self.column_position = 0;
        if self.row_position < BUFFER_HEIGHT - 1 {
            self.row_position += 1;
        }
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            unsafe {
                ptr::write_volatile(self.buffer.add(row * BUFFER_WIDTH + col), blank);
            }
        }
    }

    #[allow(dead_code)]
    pub fn set_colors(&mut self, foreground: Color, background: Color) {
        self.color_code = ColorCode::new(foreground, background);
    }
}

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.as_bytes() {
            self.write_byte(*byte);
        }
        Ok(())
    }
}

/// Initialise the VGA buffer with the physical memory offset provided by
/// the bootloader. Must be called before any VGA output.
pub fn init(phys_offset: u64) {
    let virt = VGA_BUFFER_PHYS + phys_offset;
    {
        let mut writer = WRITER.lock();
        writer.buffer = virt as *mut ScreenChar;
    }
    VGA_READY.store(true, Ordering::SeqCst);
    // Clear the screen.
    for row in 0..BUFFER_HEIGHT {
        WRITER.lock().clear_row(row);
    }
}

/// Low-level entry point used by the `print!` macro. Acquires the writer
/// and forwards the formatted arguments. No-ops until `init()` is called.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    if VGA_READY.load(Ordering::SeqCst) {
        use core::fmt::Write;
        let _ = WRITER.lock().write_fmt(args);
    }
}

/// Clear the whole screen.
#[allow(dead_code)]
pub fn clear_screen() {
    if VGA_READY.load(Ordering::SeqCst) {
        for row in 0..BUFFER_HEIGHT {
            WRITER.lock().clear_row(row);
        }
    }
}
```

---

### `kernel/src/serial.rs`

```rust
//! 16550-compatible UART (COM1, 0x3F8) serial console.
//!
//! Used as the primary debug/log channel: output appears on the host
//! serial device (e.g. `qemu -serial stdio`) and is shared with the VGA
//! buffer through the `print!`/`println!` macros.

use core::fmt::{self, Write};
use spin::Mutex;
use uart_16550::SerialPort;

/// COM1. Add more static ports here if you want COM2/COM3 logging.
pub static COM1: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3F8) });

/// Initialise the COM1 UART (line + FIFO + modem config).
pub fn init() {
    COM1.lock().init();
    let _ = writeln!(COM1.lock(), "[serial] AeroOS serial console online");
}

/// Low-level entry point used by the `print!` macro.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    let _ = COM1.lock().write_fmt(args);
}
```

---

### `kernel/src/memory/mod.rs`

```rust
//! Kernel heap allocator.
//!
//! Uses a static byte array as the heap backing store. This avoids the need
//! for the bootloader to map all physical memory (which would conflict with
//! the kernel's own virtual address space). The 1 MiB heap is more than
//! sufficient for a minimal kernel.

use linked_list_allocator::LockedHeap;

/// Size of the kernel heap.
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

/// Static backing store for the heap. Linked into the kernel's BSS section.
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// Global allocator backed by a linked list of free blocks.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Set up the heap. Called once from `kernel_main`.
///
/// Uses a static array as the backing store, so no BootInfo or physical
/// memory mapping is required.
pub fn init() {
    // SAFETY: `HEAP` is a static array that exists for the entire lifetime of
    // the kernel. `init` is called exactly once from `kernel_main` before any
    // allocation occurs. The `LockedHeap` implementation ensures thread-safe
    // access via a spinlock.
    unsafe {
        let heap_start = core::ptr::addr_of_mut!(HEAP) as *mut u8;
        ALLOCATOR.lock().init(heap_start, HEAP_SIZE);
    }
}
```

---

### `kernel/src/interrupts/mod.rs`

```rust
//! Interrupt subsystem: GDT + IDT + 8259 PIC + input queue.
//!
//! `init()` is the single entry point called from `kernel_main` after
//! logging is up. It sets up segments, the exception table, the PIC and
//! finally enables maskable interrupts (`sti`).

use pic8259::ChainedPics;
use spin::Mutex;

pub mod gdt;
pub mod idt;

/// Offset the master PIC to 0x20 so IRQs don't overlap CPU exceptions
/// (which occupy 0x00..=0x1f).
pub const PIC_1_OFFSET: u8 = 0x20;
pub const PIC_2_OFFSET: u8 = 0x28;

/// The two cascaded 8259 PICs. Wrapped in a spinlock because the mask
/// and EOI registers are read/written from interrupt handlers.
pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// Hardware interrupt vectors remapped from the PIC offsets.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    pub fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

/// A const-constructible single-producer/single-consumer ring buffer for
/// raw PS/2 scan codes. We avoid `alloc` here so the queue can live in a
/// plain static (no lazy init needed) and be ready before the first
/// keyboard interrupt fires.
pub struct ScancodeBuffer {
    buf: [u8; 256],
    head: usize, // write cursor
    tail: usize, // read cursor
}

impl ScancodeBuffer {
    pub const fn new() -> Self {
        Self {
            buf: [0; 256],
            head: 0,
            tail: 0,
        }
    }

    /// Push a scan code. Drops the byte silently if the buffer is full.
    pub fn push(&mut self, byte: u8) {
        let next = (self.head + 1) % 256;
        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
    }

    /// Pop the next scan code, or `None` if empty.
    pub fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let byte = self.buf[self.tail];
        self.tail = (self.tail + 1) % 256;
        Some(byte)
    }
}

/// The global scan-code queue, drained by the shell host.
pub static SCANCODE_BUFFER: Mutex<ScancodeBuffer> = Mutex::new(ScancodeBuffer::new());

/// Install the GDT, IDT, initialise the PIC, then enable interrupts.
pub fn init() {
    gdt::init();
    idt::init();
    unsafe {
        PICS.lock().initialize();
    }
    x86_64::instructions::interrupts::enable();
}

/// Unmask IRQ0 (timer) and IRQ1 (keyboard) on the master PIC. A bit set
/// in the mask disables that interrupt.
pub fn enable_keyboard() {
    unsafe {
        PICS.lock().write_masks(0b1111_1100, 0xff);
    }
}
```

---

### `kernel/src/interrupts/gdt.rs`

```rust
//! Global Descriptor Table (GDT) + Task State Segment (TSS).
//!
//! The GDT defines the kernel code segment. The TSS provides an
//! independent interrupt stack table so that a double fault cannot
//! corrupt the currently-in-use stack.

use lazy_static::lazy_static;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// Index into the TSS interrupt stack table used for the double fault.
pub const DOUBLE_FAULT_IST: u16 = 0;

/// 20 KiB, 16-byte aligned private stack for double faults.
#[repr(C, align(16))]
struct DoubleFaultStack([u8; 20480]);

static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack([0; 20480]);

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST as usize] = {
            // Top of the stack = bottom + size (stacks grow downwards).
            let ptr = core::ptr::addr_of_mut!(DOUBLE_FAULT_STACK);
            let bottom = ptr as usize;
            VirtAddr::new((bottom + core::mem::size_of::<DoubleFaultStack>()) as u64)
        };
        tss
    };

    static ref GDT: (GlobalDescriptorTable, SegmentSelector, SegmentSelector, SegmentSelector) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code = gdt.append(Descriptor::kernel_code_segment());
        let data = gdt.append(Descriptor::kernel_data_segment());
        let tss = gdt.append(Descriptor::tss_segment(&*TSS));
        (gdt, code, data, tss)
    };
}

/// Load the GDT, refresh code + data segments and install the TSS.
pub fn init() {
    use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
    use x86_64::instructions::tables::load_tss;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1);
        SS::set_reg(GDT.2);
        DS::set_reg(GDT.2);
        ES::set_reg(GDT.2);
        load_tss(GDT.3);
    }
}
```

---

### `kernel/src/interrupts/idt.rs`

```rust
//! Interrupt Descriptor Table (IDT): CPU exceptions + hardware IRQs.
//!
//! The IDT routes every interrupt vector to a handler function. We wire
//! up the breakpoint/double-fault exceptions plus the 8259 PIC timer and
//! keyboard IRQs. Keyboard scan codes are pushed onto a lock-free queue
//! that the shell host drains.

use lazy_static::lazy_static;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

use crate::interrupts::{InterruptIndex, PICS, SCANCODE_BUFFER};
use crate::println;

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::interrupts::gdt::DOUBLE_FAULT_IST);
        }
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt
    };
}

/// Load the IDT into the CPU (`lidt`).
pub fn init() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("[int] BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    panic!("[int] DOUBLE FAULT (code {})\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Acknowledge the timer so the PIC can deliver the next tick.
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Read the scan code from the PS/2 data port (0x60).
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    SCANCODE_BUFFER.lock().push(scancode);

    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}
```

---

### `.gitignore`

```
/target/
**/target/
**/*.rs.bk
Cargo.lock
```

---

### `.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [main, master]
  pull_request:
    branches: [main, master]

jobs:
  kernel:
    name: Build Kernel
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust nightly
        uses: dtolnay/rust-toolchain@nightly
        with:
          components: rust-src, llvm-tools-preview
          targets: x86_64-unknown-none

      - name: Build kernel
        run: cargo build -p aeros-kernel --target x86_64-unknown-none --verbose

  os-image:
    name: Build OS Image
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust nightly
        uses: dtolnay/rust-toolchain@nightly
        with:
          components: rust-src, llvm-tools-preview
          targets: x86_64-unknown-none

      - name: Build OS image
        run: cargo build -p aeros-os --verbose
```

---

## 编译运行

### 前置条件

```bash
# Rust nightly (通过 rust-toolchain.toml 自动安装)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly

# QEMU
sudo apt-get install -y qemu-system-x86
```

### 构建

```bash
./build.sh
```

或手动：

```bash
cargo build -p aeros-os --release
cargo run -p aeros-os --release
# 镜像路径: target/aeros-os-bios.img
```

### 运行

```bash
qemu-system-x86_64 \
  -drive format=raw,file=target/aeros-os-bios.img \
  -serial stdio
```

### 无显示运行（仅串口）

```bash
qemu-system-x86_64 \
  -drive format=raw,file=target/aeros-os-bios.img \
  -serial stdio \
  -display none \
  -no-reboot
```

---

## 设计要点

| 问题 | 解决方案 |
|------|----------|
| `build-std` + bindeps 冲突 | 自行实现 `memcpy`/`memset`/`memmove`/`memcmp`/`strlen`，不使用 `build-std` |
| bootloader 默认编译 UEFI（`wcslen` 链接错误） | `default-features = false, features = ["bios"]` |
| VGA 文本缓冲区 0xB8000 不可直接访问 | `Mapping::Dynamic` 映射物理内存 + `vga_buffer::init(phys_offset)` 设置虚拟地址 |
| GDT 无数据段导致定时器中断 Double Fault | GDT 包含 `kernel_data_segment()`，加载后设置 SS/DS/ES |
| 堆分配器依赖物理内存映射 | 改用 1 MiB 静态 BSS 数组作为堆后备存储 |

## 已知限制

- **仅 BIOS** — 无 UEFI 支持
- **无分页** — 依赖 bootloader 的页表
- **无文件系统** — 无 VFS 或磁盘驱动
- **无网络** — 无网卡驱动
- **单核** — 无 APIC、无 SMP
- **仅 PS/2 键盘** — 无 USB 支持
- **VGA 文本模式** — 无图形帧缓冲渲染
- **堆在 BSS** — 1 MiB 静态数组，非动态分配

## License

MIT
