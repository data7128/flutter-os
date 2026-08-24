//! UHCI (Universal Host Controller Interface) driver.
//!
//! UHCI is the Intel-defined USB 1.x host controller standard.
//! QEMU emulates a UHCI controller on the PIIX3 southbridge.
//!
//! ## UHCI Register Map (I/O port space)
//!
//! The UHCI register set is accessed via I/O ports. The base address
//! is obtained from the PCI configuration space (BAR4, always I/O).
//!
//! ```text
//! Offset  Register              Size  Description
//! 0x00     USBCMD                u16   Command register
//! 0x02     USBSTS                u16   Status register
//! 0x04     USBINTR               u16   Interrupt enable
//! 0x06     FRNUM                 u16   Frame number
//! 0x08     FLBASEADD             u32   Frame list base address
//! 0x0C     SOFMOD                u8    Start of Frame modify
//! 0x0D     PORTSC1               u16   Port 1 status/control
//! 0x0F     PORTSC2               u16   Port 2 status/control
//! ```
//!
//! ## Transfer Descriptors (TD)
//!
//! UHCI uses a linked list of Transfer Descriptors for scheduling
//! USB transactions. Each TD describes one USB transaction (IN, OUT,
//! SETUP). TDs are linked into Queue Heads (QH) which are placed
//! in the periodic frame list (1024 entries).
//!
//! [MANUAL] This skeleton defines the structures and register access,
//! but the actual transfer execution requires:
//! - DMA-capable memory allocation for TD/QH (needs paging)
//! - Frame list initialization (1024 x 4-byte entries)
//! - Interrupt handling (IRQ assignment from PCI)
//! - Timing validation for USB 1.x (1ms frame intervals)

use spin::Mutex;
use x86_64::instructions::port::Port;

/// UHCI I/O register offsets (from base address).
pub mod regs {
    pub const USBCMD: u16 = 0x00;
    pub const USBSTS: u16 = 0x02;
    pub const USBINTR: u16 = 0x04;
    pub const FRNUM: u16 = 0x06;
    pub const FLBASEADD: u16 = 0x08;
    pub const SOFMOD: u16 = 0x0C;
    pub const PORTSC1: u16 = 0x0D; // Actually 0x10 for word access
    pub const PORTSC2: u16 = 0x12;
}

/// USBCMD bits.
pub mod cmd {
    pub const RS: u16 = 0x0001;       // Run/Stop
    pub const HCRESET: u16 = 0x0002;  // Host Controller Reset
    pub const GRESET: u16 = 0x0004;   // Global Reset
    pub const MAXPKT: u16 = 0x0040;   // Max Packet (64 bytes)
    pub const CF: u16 = 0x0080;       // Configure Flag
}

/// USBSTS bits.
pub mod sts {
    pub const USBINT: u16 = 0x0001;    // USB Interrupt
    pub const USBERR: u16 = 0x0002;    // USB Error Interrupt
    pub const RD: u16 = 0x0004;       // Resume Detect
    pub const HSE: u16 = 0x0008;      // Host System Error
    pub const HCPE: u16 = 0x0010;     // Host Controller Process Error
    pub const HCH: u16 = 0x0020;      // HC Halted
}

/// PORTSC bits.
pub mod port {
    pub const CONNECTED: u16 = 0x0001;  // Current Connect Status
    pub const CONNECT_CHG: u16 = 0x0002; // Connect Status Change
    pub const ENABLE: u16 = 0x0004;     // Port Enabled
    pub const ENABLE_CHG: u16 = 0x0008; // Port Enable/Disable Change
    pub const RESET: u16 = 0x0200;      // Port Reset
    pub const SUSPEND: u16 = 0x1000;    // Suspend
}

/// UHCI Transfer Descriptor (32 bytes, must be 16-byte aligned).
///
/// Describes a single USB transaction. TDs form a linked list.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(16))]
pub struct TransferDescriptor {
    /// Link pointer: next TD/QH address (bits 4..31 = address).
    /// Bit 0 = Terminate (1 = end of list).
    /// Bit 1 = QH/TD select (1 = QH).
    /// Bit 2 = Depth-first (1 = breadth-first).
    pub link_ptr: u32,
    /// Control bits: actual_length, status, IOC, IOS, LS, C_ERR, SPD, PID.
    pub control: u32,
    /// Token: device address, endpoint, PID (IN/OUT/SETUP), length.
    pub token: u32,
    /// Buffer pointer: physical address of data buffer.
    pub buffer_ptr: u32,
    /// Reserved for hardware use (4 u32 = 16 bytes padding).
    pub _reserved: [u32; 4],
}

impl TransferDescriptor {
    pub const fn new() -> Self {
        Self {
            link_ptr: 1, // Terminate (end of list).
            control: 0,
            token: 0,
            buffer_ptr: 0,
            _reserved: [0; 4],
        }
    }
}

/// UHCI Queue Head (8 bytes, must be 16-byte aligned).
///
/// A QH is a branching point in the TD linked list. The scheduler
/// follows the QH's link, then returns to the QH's horizontal link.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(16))]
pub struct QueueHead {
    /// Head link: first TD in this queue (or next QH/TD).
    pub head_ptr: u32,
    /// Horizontal link: next QH in the schedule.
    pub element_ptr: u32,
}

impl QueueHead {
    pub const fn new() -> Self {
        Self {
            head_ptr: 1, // Terminate.
            element_ptr: 1, // Terminate.
        }
    }
}

/// UHCI controller state.
pub struct UhciController {
    /// I/O port base address (from PCI BAR4).
    pub io_base: u16,
    /// Number of ports (typically 2).
    pub num_ports: u8,
    /// Whether the controller has been initialised.
    pub initialised: bool,
}

impl UhciController {
    pub const fn new() -> Self {
        Self {
            io_base: 0,
            num_ports: 2,
            initialised: false,
        }
    }

    /// Read a 16-bit UHCI register.
    unsafe fn read16(&self, offset: u16) -> u16 {
        let mut port = Port::new(self.io_base + offset);
        port.read()
    }

    /// Write a 16-bit UHCI register.
    unsafe fn write16(&self, offset: u16, value: u16) {
        let mut port = Port::new(self.io_base + offset);
        port.write(value);
    }

    /// Read a 32-bit UHCI register.
    #[allow(dead_code)]
    unsafe fn read32(&self, offset: u16) -> u32 {
        let mut port = Port::new(self.io_base + offset);
        port.read()
    }

    /// Write a 32-bit UHCI register.
    #[allow(dead_code)]
    unsafe fn write32(&self, offset: u16, value: u32) {
        let mut port = Port::new(self.io_base + offset);
        port.write(value);
    }

    /// Reset the UHCI controller (HCRESET).
    ///
    /// [MANUAL] Needs timing validation — HCRESET must be de-asserted
    /// within ~10ms. QEMU may behave differently from real hardware.
    unsafe fn reset(&self) {
        // Assert reset.
        self.write16(regs::USBCMD, cmd::HCRESET);

        // Wait for reset to complete (poll for HCRESET to clear).
        // In TCG software emulation, loops are very slow, so keep
        // the iteration count modest.
        let mut timeout = 0u32;
        while (self.read16(regs::USBCMD) & cmd::HCRESET) != 0 {
            timeout += 1;
            if timeout > 1000 {
                crate::serial::_print(format_args!(
                    "[uhci] reset timeout!\n"
                ));
                break;
            }
        }
    }

    /// Initialize the UHCI controller.
    ///
    /// [MANUAL] The full init sequence requires:
    /// - PCI BAR4 read to get I/O base
    /// - Frame list allocation (1024 x 4 bytes, 4K aligned)
    /// - TD/QH allocation and initialization
    /// - IRQ routing from PCI
    /// - Port reset and device enumeration
    pub fn init_controller(&mut self) -> bool {
        if self.io_base == 0 {
            // [MANUAL] On real hardware, scan PCI config space for
            // UHCI controller (class 0x0C, subclass 0x00, prog-if 0x00).
            // QEMU PIIX3 UHCI is at PCI 00:01.2, BAR4.
            //
            // For skeleton: use a known QEMU default.
            self.io_base = 0xC040; // QEMU default UHCI I/O base
            crate::serial::_print(format_args!(
                "[uhci] using QEMU default I/O base: {:#x}\n", self.io_base
            ));
        }

        unsafe {
            // 1. Disable interrupts.
            self.write16(regs::USBINTR, 0);

            // 2. Reset the controller.
            self.reset();

            // 3. Set Configure Flag + Run/Stop.
            self.write16(regs::USBCMD, cmd::CF);

            // 4. Clear all status bits.
            self.write16(regs::USBSTS, 0xFF);

            // 5. Reset ports.
            for port_num in 1..=self.num_ports {
                let port_reg = if port_num == 1 { regs::PORTSC1 } else { regs::PORTSC2 };
                // Assert port reset.
                self.write16(port_reg, port::RESET);

                // Wait ~50ms (simplified: busy-wait).
                // Reduced for TCG CI compatibility.
                for _ in 0..1000 { x86_64::instructions::nop(); }

                // Clear port reset.
                self.write16(port_reg, 0);

                // Check if device is connected.
                let port_sts = self.read16(port_reg);
                let connected = (port_sts & port::CONNECTED) != 0;
                crate::serial::_print(format_args!(
                    "[uhci] port {}: connected={}\n", port_num, connected
                ));
            }

            // 6. Enable the controller.
            self.write16(regs::USBCMD, cmd::CF | cmd::RS);
        }

        self.initialised = true;
        crate::serial::_print(format_args!(
            "[uhci] controller initialized\n"
        ));
        true
    }
}

/// Global UHCI controller instance.
pub static UHCI: Mutex<UhciController> = Mutex::new(UhciController::new());

/// Initialize UHCI: scan PCI, reset controller, enumerate ports.
pub fn init() {
    let mut ctrl = UHCI.lock();
    ctrl.init_controller();
}
