//! USB subsystem — UHCI host controller driver + HID device support.
//!
//! ## Limitations
//! - **Only UHCI** (Universal Host Controller Interface): supports
//!   Intel PIIX3/PIIX4 USB controllers as emulated by QEMU.
//! - **No EHCI/XHCI**: EHCI (USB 2.0) and XHCI (USB 3.0) are not supported.
//! - **Only HID keyboard/mouse**: no USB mass storage, no USB audio.
//! - **No USB hubs**: direct device connection only (QEMU provides this).
//!
//! ## Architecture
//!
//! ```text
//! USB subsystem
//! ├── uhci.rs   — UHCI controller: PCI enum, I/O port regs, frame list,
//! │               transfer descriptors (TD), queue heads (QH)
//! ├── hid.rs    — HID keyboard/mouse: interrupt transfers, boot protocol
//! └── mod.rs    — unified input dispatch to kernel input subsystem
//! ```
//!
//! [MANUAL] The UHCI driver skeleton is generated but requires
//! extensive hardware debugging. Transfer descriptor management,
//! frame list scheduling, and interrupt handling need careful
//! timing validation on real hardware / QEMU.

pub mod hid;
pub mod uhci;

/// Initialize the USB subsystem.
///
/// 1. Scan PCI bus for UHCI controllers
/// 2. Initialize the UHCI controller
/// 3. Enumerate USB devices
/// 4. Set up HID keyboard/mouse drivers
pub fn init() {
    crate::serial::_print(format_args!("[usb] initializing USB subsystem\n"));

    // Initialize UHCI controller.
    uhci::init();

    // Enumerate devices and set up HID drivers.
    hid::init();

    crate::serial::_print(format_args!("[usb] USB subsystem ready\n"));
}
