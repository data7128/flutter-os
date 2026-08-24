# flutter-os

A minimal x86_64 bare-metal operating system kernel written in Rust, booted via the [`bootloader`](https://crates.io/crates/bootloader) crate (v0.11) with BIOS support, runnable in QEMU.

## Features

- **GDT** — Global Descriptor Table with kernel code segment, data segment, and TSS (double-fault IST)
- **IDT** — Interrupt Descriptor Table: breakpoint, double fault, timer, keyboard
- **8259A PIC** — Remapped, masked timer + keyboard IRQs
- **Heap allocation** — 1 MiB kernel heap via `linked_list_allocator` (static backing array)
- **VGA text output** — 80x25 text buffer at physical 0xB8000, mapped via bootloader's physical memory offset
- **COM1 serial** — 16550 UART at 0x3F8 for debug/logging
- **PS/2 keyboard** — Set-1 scancode to ASCII echo

## Project Structure

```
flutter-os/
├── .cargo/config.toml       # bindeps feature (nightly)
├── .github/workflows/ci.yml # CI: build kernel + OS image
├── .gitignore
├── rust-toolchain.toml      # nightly + rust-src + llvm-tools + x86_64-unknown-none
├── Cargo.toml               # workspace root: OS image builder (std)
├── build.rs                 # BiosBoot::create_disk_image()
├── build.sh                 # one-command build script
├── src/main.rs              # copies .img to target/
├── LICENSE
├── README.md
└── kernel/                  # no_std kernel crate
    ├── Cargo.toml
    └── src/
        ├── lib.rs           # kernel_main, print macros, bootloader config, scancode map
        ├── main.rs          # entry_point! + panic_handler
        ├── mem.rs           # memcpy/memset/memmove/memcmp/strlen (no build-std needed)
        ├── vga_buffer.rs    # VGA text mode (0xB8000) with physical offset init
        ├── serial.rs        # COM1 16550 UART
        ├── memory/mod.rs    # heap allocator (static array, 1 MiB)
        └── interrupts/
            ├── mod.rs       # 8259 PIC + scancode ring buffer
            ├── gdt.rs       # GDT + TSS (code + data segments)
            └── idt.rs       # IDT exception + IRQ handlers
```

## Prerequisites

```bash
# Rust nightly (components installed via rust-toolchain.toml)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly

# QEMU (for running the image)
sudo apt-get install -y qemu-system-x86
```

## Build

```bash
./build.sh
```

Or manually:

```bash
# Build the bootable BIOS disk image
cargo build -p aeros-os --release
cargo run -p aeros-os --release

# Image appears at: target/aeros-os-bios.img
```

## Run in QEMU

```bash
qemu-system-x86_64 \
  -drive format=raw,file=target/aeros-os-bios.img \
  -serial stdio
```

You should see boot messages on the serial console and VGA screen, then keyboard input is echoed to both.

## Design Notes

- **No `build-std`**: The kernel provides its own `memcpy`/`memset`/`memmove`/`memcmp`/`strlen` implementations in `mem.rs`, avoiding the need for `build-std-features=compiler-builtins-mem`.
- **Bootloader config**: Requests `Mapping::Dynamic` for physical memory. The VGA text buffer at physical `0xB8000` is accessed at `0xB8000 + physical_memory_offset` (set during `vga_buffer::init()`).
- **Heap**: Uses a 1 MiB static byte array in BSS as the heap backing store, avoiding dependence on physical memory mapping for the allocator.
- **GDT**: Includes both kernel code and data segments. Setting SS/DS/ES to the data segment after loading the GDT prevents a double fault when the timer interrupt fires (the bootloader's old segment selectors become invalid).
- **BIOS only**: The `bootloader` dependency uses `default-features = false, features = ["bios"]` to avoid compiling UEFI components (which require `wcslen` and other libc functions not available in `no_std`).

## Known Limitations

- **BIOS only** — no UEFI support
- **No paging** — relies on the bootloader's page tables; no custom page table management
- **No filesystem** — the kernel has no VFS or disk driver
- **No network** — no NIC driver
- **No multi-core** — single-core only (no APIC, no SMP)
- **PS/2 only** — no USB keyboard/mouse support
- **VGA text mode** — no graphical framebuffer rendering
- **Heap in BSS** — the 1 MiB heap is a static array, not dynamically allocated from physical memory regions

## License

MIT
