# AeroOS

An experimental x86_64 bare-metal operating system kernel written in Rust, with a user-mode window manager, Flutter system shell, USB input, and .aero native app format.

> **Status**: Experimental prototype. No network stack, limited software ecosystem, software rendering only.

## Architecture

```
┌───────────────────────────────────────────────────────────┐
│  User-Space (Ring 3)                                      │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐ │
│  │ Flutter  │ │ Window   │ │ Sysutils │ │ .aero Apps   │ │
│  │ Shell    │ │ Manager  │ │ ls/cat/  │ │ (Terminal,   │ │
│  │ (M3 desk)│ │ (composit)│ │ ps/kill  │ │  Files, etc) │ │
│  └─────┬────┘ └─────┬────┘ └─────┬────┘ └──────┬───────┘ │
│        │             │           │              │         │
│  ┌─────▼─────────────▼───────────▼──────────────▼───────┐ │
│  │  Flutter Adapter (WM surface + input event bridge)   │ │
│  └──────────────────────────────────────────────────────┘ │
│        │ fb_commit │ poll_input │ kill │ exec │ getpid    │
├────────┼───────────┼────────────┼──────┼──────┼──────────┤
│  Kernel (Ring 0)                                          │
│  ┌──────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐│
│  │ Process  │ │ Signal │ │ OOM    │ │ Perm   │ │ ELF    ││
│  │ Table    │ │ (KILL) │ │ Handler│ │ Model  │ │ Loader ││
│  └──────────┘ └────────┘ └────────┘ └────────┘ └────────┘│
│  ┌──────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐│
│  │ Framebuf │ │ PS/2   │ │ USB    │ │ FAT32  │ │ .aero  ││
│  │ Graphics │ │ KB+Mse │ │ UHCI   │ │ (RO)   │ │ Format ││
│  └──────────┘ └────────┘ └────────┘ └────────┘ └────────┘│
│  ┌──────────┐ ┌────────┐ ┌────────┐ ┌────────┐              │
│  │ GDT/IDT  │ │ 8259PIC│ │ Heap   │ │ Serial │              │
│  └──────────┘ └────────┘ └────────┘ └────────┘              │
└───────────────────────────────────────────────────────────┘
```

## Implemented Features

| Subsystem | Status | Marker | Description |
|-----------|--------|--------|-------------|
| GDT + TSS | Done | `[OK] GDT` | Kernel code/data segments, double-fault IST |
| IDT | Done | `[OK] IDT` | Exception + IRQ handlers (timer, keyboard, mouse) |
| 8259 PIC | Done | `[OK] PIC` | Remapped, IRQ0-15 unmasked |
| Heap | Done | `[OK] HEAP` | 1 MiB static-backed allocator |
| PS/2 Keyboard | Done | `[OK] KEYBOARD` | Set-1 scancode to ASCII |
| PS/2 Mouse | Done | `[OK] MOUSE` | IRQ12, 3-byte packet parsing |
| Framebuffer | Done | `[OK] GRAPHICS` | Linear framebuffer, draw primitives |
| Time | Done | `[OK] TIME` | PIT tick counter, clock_gettime |
| Syscalls | Done | `[OK] SYSCALLS` | int 0x80 dispatch (13 syscalls) |
| Flutter Adapter | Skeleton | `[OK] FLUTTER_ADAPTER` | WM surface + input bridge |
| Window Manager | Skeleton | `[OK] WINDOW_MANAGER` | Ring3 software compositor |
| ELF Loader | Skeleton | `[OK] EXEC_LOADER` | ELF64 header parsing + validation |
| Flutter Shell | Skeleton | `[OK] FLUTTER_SHELL` | Material3 desktop (software rendered) |
| Signal | Skeleton | `[OK] SIGNAL_SUBSYS` | SIGKILL/SIGTERM only |
| OOM Handler | Skeleton | `[OK] OOM_HANDLER` | Reap zombies, kill memory hogs |
| UHCI USB | Skeleton | `[OK] UHCI_USB` | UHCI controller driver skeleton |
| USB HID Input | Skeleton | `[OK] USB_HID_INPUT` | Keyboard/mouse boot protocol |
| .aero Format | Skeleton | `[OK] AERO_APP_FORMAT` | Native app package format |
| Permission | Skeleton | `[OK] PERMISSION_SUBSYS` | Kernel/System/User privilege levels |
| Ring3 Usermode | Pending | `[PENDING] USERMODE` | Requires TSS + page tables + iretq |
| Scheduler | Pending | `[PENDING] SCHEDULER` | Requires context switching |
| Fork/Exec | Pending | `[PENDING] FORK_EXEC` | Requires process address space dup |

## Syscall Table

| # | Name | Args | Description |
|---|------|------|-------------|
| 1 | open | path, flags | Open file (ENOSYS — needs FAT32) |
| 2 | read | fd, buf, count | Read from fd (stdin=PS/2) |
| 3 | write | fd, buf, count | Write to fd (stdout=serial+VGA) |
| 4 | mmap | addr, len, prot | Allocate memory (heap-backed) |
| 5 | nanosleep | ticks, rem | Sleep N PIT ticks |
| 6 | clock_gettime | clk_id, tp | Get monotonic time |
| 7 | get_framebuffer_info | info_ptr | Query fb geometry |
| 8 | fb_commit | buf, x, y, w, h | Blit buffer to framebuffer |
| 9 | poll_input | event_ptr | Poll keyboard/mouse event |
| 10 | kill | pid, signum | Send signal to process |
| 11 | exit | status | Terminate calling process |
| 12 | exec | path | Load and run ELF executable |
| 13 | getpid | — | Return current PID |

## Project Structure

```
flutter-os/
├── kernel/           # no_std kernel (Ring 0)
│   └── src/
│       ├── lib.rs           # kernel_main, boot markers
│       ├── main.rs          # entry_point! + panic handler
│       ├── process.rs      # process table (PID, state, FD, signals)
│       ├── exec.rs          # ELF64 loader + exec syscall
│       ├── boot_service.rs  # launch WM + Flutter Shell
│       ├── signal.rs        # SIGKILL/SIGTERM delivery
│       ├── oom.rs           # OOM recovery (reap + kill)
│       ├── perm.rs          # permission model (kernel/system/user)
│       ├── aero_format.rs   # .aero app package format
│       ├── syscalls/        # int 0x80 dispatch (13 syscalls)
│       │   ├── mod.rs       # dispatch table
│       │   ├── fd.rs        # file descriptor table
│       │   ├── input.rs     # InputEvent struct + poll
│       │   └── time.rs      # PIT tick counter
│       ├── interrupts/       # GDT + IDT + PIC + input queues
│       │   ├── mod.rs       # PIC + scancode buffer
│       │   ├── gdt.rs       # segments + TSS
│       │   ├── idt.rs       # exception + IRQ handlers
│       │   └── mouse.rs    # PS/2 mouse driver
│       ├── usb/             # USB subsystem
│       │   ├── mod.rs       # init
│       │   ├── uhci.rs      # UHCI host controller
│       │   └── hid.rs       # HID keyboard/mouse
│       ├── graphics/        # framebuffer drawing
│       ├── memory/          # heap allocator
│       ├── serial.rs        # COM1 UART
│       ├── vga_buffer.rs    # VGA text mode
│       └── shell_host.rs    # kernel shell (echo)
├── wm/               # Window Manager (Ring 3, no_std)
│   └── src/
│       ├── lib.rs           # init + event loop
│       ├── window.rs        # Window + WindowList + Z-order
│       ├── compositor.rs    # off-screen buffer + blit
│       ├── cursor.rs        # software mouse cursor
│       ├── input_router.rs  # event dispatch + drag
│       ├── flutter_api.rs   # Flutter surface interface
│       └── syscalls.rs      # syscall wrappers
├── flutter_shell/    # System desktop (Ring 3, no_std)
│   └── src/
│       ├── lib.rs           # init + event loop
│       ├── render.rs        # software pixel renderer
│       ├── widgets.rs       # Material3 TaskBar/Launcher/Clock
│       └── syscalls.rs      # syscall wrappers
├── flutter_adapter/   # Flutter Engine bridge (Ring 3)
│   └── src/
│       ├── lib.rs           # init + run loop
│       ├── embedder.rs      # Flutter Engine C API types
│       ├── framebuffer.rs   # mmap canvas
│       ├── input.rs         # PS/2 → Flutter event
│       └── syscalls.rs      # syscall wrappers
├── sysutils/          # Debug tools (Ring 3, no_std)
│   └── src/
│       ├── lib.rs           # busybox-style dispatch
│       ├── syscalls.rs      # syscall wrappers
│       └── commands/        # ls, cat, ps, kill
├── ci/check_boot.sh  # CI boot marker validation
├── docs/             # Flutter porting research
├── Cargo.toml        # workspace (5 members)
└── build.sh          # one-command build
```

## Known Limitations

- **Experimental prototype** — not suitable for production use
- **No network stack** — no NIC driver, no TCP/IP, no sockets
- **No Ring3 usermode** — kernel runs all code in Ring 0; Ring3 context switch is pending
- **Software rendering only** — no GPU, no hardware acceleration; all graphics are CPU-rendered
- **PS/2 + USB UHCI only** — no EHCI/XHCI; no USB mass storage
- **FAT32 read-only** — no write support, no journaling
- **No paging** — relies on bootloader's identity-mapped page tables
- **No multi-core** — single-core only, no APIC, no SMP
- **No virtual memory** — all processes share kernel address space (until paging is implemented)
- **.aero format has no security** — no signatures, no checksums, no integrity verification
- **Signal subset only** — SIGKILL/SIGTERM; no SIGINT/SIGHUP/SIGSEGV/sigaction
- **Limited software ecosystem** — no package manager, no standard library

## Hardware Support

| Device | Driver | Status |
|--------|--------|--------|
| 16550 UART (COM1) | serial.rs | Done |
| 8259A PIC | interrupts/mod.rs | Done |
| PS/2 Keyboard (8042) | interrupts/idt.rs | Done |
| PS/2 Mouse (8042) | interrupts/mouse.rs | Done |
| Linear Framebuffer | graphics/mod.rs | Done |
| ATA Disk (FAT32 RO) | (planned) | Pending |
| UHCI USB Controller | usb/uhci.rs | Skeleton |
| USB HID Keyboard | usb/hid.rs | Skeleton |
| USB HID Mouse | usb/hid.rs | Skeleton |

## Build and Run

### Prerequisites

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly
sudo apt-get install -y qemu-system-x86
```

### Build

```bash
./build.sh
```

### Run in QEMU

```bash
qemu-system-x86_64 \
  -drive format=raw,file=target/aeros-os-bios.img \
  -serial stdio \
  -device usb-uhci \
  -usbdevice keyboard \
  -usbdevice mouse
```

### CI

GitHub Actions runs `ci/check_boot.sh` which boots the OS in QEMU and validates all `[OK]` serial markers.

## License

MIT
