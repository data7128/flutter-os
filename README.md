# AeroOS

A minimal x86_64 operating system built with **Rust runtime** and a **Flutter** desktop shell.

## Architecture

```
┌─────────────────────────────────────────────┐
│               Flutter Shell (Dart)            │
│   Material 3 desktop: taskbar, windows, apps  │
├─────────────────────────────────────────────┤
│          Flutter Engine Embedder             │
│    flutter-pi / flutter-elinux (DRM/GBM/EGL)  │
├─────────────────────────────────────────────┤
│              Rust Kernel (aeros-kernel)       │
│  GDT/IDT/PIC · Heap · Framebuffer · Shell Host│
├─────────────────────────────────────────────┤
│                 Bootloader (v0.11)           │
├─────────────────────────────────────────────┤
│                    x86_64 Hardware            │
└─────────────────────────────────────────────┘
```

### Kernel (Rust, `kernel/`)

- **Boot**: `bootloader` crate v0.11 (UEFI/BIOS), enters long mode with paging enabled
- **Segments**: GDT + TSS (double-fault IST)
- **Interrupts**: IDT with breakpoint/double-fault handlers, 8259 PIC timer + keyboard IRQs
- **Memory**: `linked_list_allocator` heap seeded from bootloader's memory map
- **Graphics**: Linear framebuffer with `put_pixel` / `fill_rect` utilities
- **Input**: PS/2 scan-code ring buffer (lock-free, no `alloc`)
- **Shell host**: Renders a minimal desktop (gradient, taskbar, window frame) to the framebuffer

### Shell (Flutter, `shell/`)

- Material 3 desktop UI with a taskbar, draggable windows, and an app launcher
- System info (uptime, live clock)
- Designed to run via `flutter-pi` or `flutter-elinux` on the DRM/GBM/EGL backend

### OS Image Builder (`os/`)

- Uses `bootloader` v0.11 artifact dependencies to produce a bootable disk image
- `cargo build` in `os/` links the kernel + bootloader into a single bootable binary

## Prerequisites

- **Rust** nightly toolchain (`rustup install nightly`)
- **Flutter** SDK 3.3+
- **QEMU** (optional, for testing the kernel image)
- **gh** CLI (for GitHub upload)

## Build & Run

### 1. Build the kernel

```bash
cd kernel
cargo build
```

### 2. Build the bootable OS image

```bash
cd os
cargo build
# The bootable image is in os/target/x86_64-unknown-none/debug/aeros-os
```

### 3. Run in QEMU

```bash
qemu-system-x86_64 -drive format=raw,file=os/target/x86_64-unknown-none/debug/aeros-os -serial stdio
```

### 4. Build the Flutter shell (host build for testing)

```bash
cd shell
flutter pub get
flutter run -d linux
```

### One-command build

```bash
./build.sh
```

## Project Structure

```
flutter-os/
├── kernel/              # Rust freestanding kernel
│   ├── Cargo.toml
│   ├── .cargo/config.toml
│   └── src/
│       ├── main.rs          # Binary entry (standalone build)
│       ├── lib.rs           # Library: kernel_main, hlt_loop, macros
│       ├── serial.rs        # 16550 UART COM1
│       ├── vga_buffer.rs    # 80x25 VGA text buffer
│       ├── graphics/        # Linear framebuffer
│       ├── interrupts/      # GDT, IDT, PIC, input queue
│       ├── memory/          # Heap allocator
│       └── shell_host.rs    # Minimal desktop renderer
├── os/                  # Bootable image builder
│   ├── Cargo.toml
│   └── src/main.rs
├── shell/               # Flutter desktop shell
│   ├── pubspec.yaml
│   ├── analysis_options.yaml
│   └── lib/main.dart
├── rust-toolchain.toml  # Nightly + components
├── build.sh             # One-command build
└── README.md
```

## Boot Flow

1. Bootloader (UEFI/BIOS) loads the kernel into memory and enters 64-bit long mode
2. Kernel `_start` → `kernel_main(boot_info)`
3. Serial console init → VGA text buffer init
4. GDT/IDT/PIC init → interrupts enabled
5. Heap allocator seeded from memory map
6. Framebuffer taken from boot info
7. Keyboard IRQ enabled
8. Shell host renders desktop to framebuffer, loops on PS/2 input

## License

MIT
