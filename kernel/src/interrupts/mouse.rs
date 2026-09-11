//! PS/2 mouse driver — hardware-level packet parsing for IRQ12.
//!
//! The PS/2 mouse sends 3-byte movement packets via the same data
//! port (0x60) as the keyboard. The controller distinguishes mouse
//! data by setting the "auxiliary output buffer full" bit (bit 5)
//! in the status register (0x64). IRQ12 fires when mouse data arrives.
//!
//! Packet format (3 bytes):
//! ```text
//! byte 0: [Yovf][Xovf][Ysig][Xsig][1][Mid][Right][Left]
//! byte 1: X delta (2's complement)
//! byte 2: Y delta (2's complement, inverted: up is negative)
//! ```
//!
//! This module is pure hardware handling — no window management,
//! no cursor rendering, no GUI logic.

use spin::Mutex;
use x86_64::instructions::port::Port;

/// Mouse state: accumulated movement deltas + current button flags.
pub struct MouseState {
    /// Button flags: bit 0 = left, bit 1 = right, bit 2 = middle.
    pub buttons: u8,
    /// X delta accumulated since last drain (positive = right).
    pub dx: i16,
    /// Y delta accumulated since last drain (positive = up, already inverted).
    pub dy: i16,
    /// 3-byte packet accumulation buffer.
    packet: [u8; 3],
    /// Current byte index within the packet (0, 1, or 2).
    packet_idx: usize,
}

impl MouseState {
    pub const fn new() -> Self {
        Self {
            buttons: 0,
            dx: 0,
            dy: 0,
            packet: [0; 3],
            packet_idx: 0,
        }
    }

    /// Process one byte received from the PS/2 mouse data port.
    ///
    /// Called from the IRQ12 interrupt handler.
    pub fn on_byte(&mut self, byte: u8) {
        // Byte 0 (the status byte) always has bit 3 set (the "Always 1" bit).
        // If it doesn't, we've lost sync — resync by treating this byte as byte 0.
        if self.packet_idx == 0 && (byte & 0x08) == 0 {
            // Not a valid first byte — discard and wait for sync.
            return;
        }

        self.packet[self.packet_idx] = byte;
        self.packet_idx += 1;

        if self.packet_idx >= 3 {
            self.complete_packet();
            self.packet_idx = 0;
        }
    }

    /// Process a complete 3-byte packet.
    fn complete_packet(&mut self) {
        let status = self.packet[0];
        let x_raw = self.packet[1] as i8 as i16;
        let y_raw = self.packet[2] as i8 as i16;

        let x_overflow = (status & 0x40) != 0;
        let y_overflow = (status & 0x80) != 0;
        let x_sign = (status & 0x10) != 0;
        let y_sign = (status & 0x20) != 0;

        // X delta
        let mut dx = x_raw;
        if x_overflow {
            dx = if x_sign { -256 } else { 255 };
        }
        // X sign is already handled by i8 → i16 cast, no extra work needed.

        // Y delta — PS/2 mouse Y is inverted (up = negative). We flip it
        // so that positive dy = up (matches screen coordinate intuition).
        let mut dy = -y_raw;
        if y_overflow {
            dy = if y_sign { 255 } else { -256 };
        }

        self.dx = self.dx.saturating_add(dx);
        self.dy = self.dy.saturating_add(dy);
        self.buttons = status & 0x07;
    }

    /// Drain accumulated movement (resets dx/dy, keeps button state).
    pub fn drain(&mut self) -> (i16, i16, u8) {
        let result = (self.dx, self.dy, self.buttons);
        self.dx = 0;
        self.dy = 0;
        result
    }
}

/// Global mouse state, protected by a spinlock.
pub static MOUSE_STATE: Mutex<MouseState> = Mutex::new(MouseState::new());

/// Drain accumulated mouse movement. Called from the `poll_input` syscall.
pub fn drain_mouse() -> (i16, i16, u8) {
    MOUSE_STATE.lock().drain()
}

// ── PS/2 controller low-level helpers ──────────────────────────────

/// Wait for the controller input buffer to be empty (ready to write).
/// Returns `false` on timeout (device unresponsive).
unsafe fn wait_write() -> bool {
    let mut cmd: Port<u8> = Port::new(0x64);
    for _ in 0..10_000_000 {
        let s = cmd.read();
        if (s & 0x02) == 0 {
            return true;
        }
    }
    false
}

/// Wait for the controller output buffer to be full (data ready to read).
/// Returns `false` on timeout (device unresponsive).
unsafe fn wait_read() -> bool {
    let mut cmd: Port<u8> = Port::new(0x64);
    for _ in 0..10_000_000 {
        let s = cmd.read();
        if (s & 0x01) != 0 {
            return true;
        }
    }
    false
}

/// Send a command byte to the PS/2 controller (port 0x64).
unsafe fn send_controller_cmd(cmd: u8) {
    if !wait_write() {
        return;
    }
    let mut port: Port<u8> = Port::new(0x64);
    port.write(cmd);
}

/// Send a command byte to the mouse device (via 0xD4 prefix + port 0x60).
/// Waits for and discards the mouse ACK (0xFA). Returns `false` on timeout.
unsafe fn send_mouse_cmd(cmd: u8) -> bool {
    send_controller_cmd(0xD4); // "write to auxiliary device"
    if !wait_write() {
        return false;
    }
    let mut data: Port<u8> = Port::new(0x60);
    data.write(cmd);
    if !wait_read() {
        return false;
    }
    let mut data: Port<u8> = Port::new(0x60);
    let _ack = data.read(); // Mouse ACKs with 0xFA
    true
}

/// Initialise the PS/2 mouse: enable auxiliary port, set config,
/// reset mouse, enable packet streaming.
///
/// **Hardware-only** — no GUI logic here.
///
/// The whole sequence runs with interrupts disabled: every PS/2
/// controller reply (ACK, config byte) lands in the 0x60 output buffer
/// and is consumed synchronously here. If IRQs were on, a keyboard IRQ1
/// could steal the byte (QEMU raises IRQ1 for controller replies), the
/// OBF flag gets cleared by the keyboard handler, and our wait_read
/// would spin forever. After setup we re-enable IRQs and let IRQ12
/// deliver mouse packets.
pub fn init() {
    // Talk to the controller synchronously; see doc comment above.
    x86_64::instructions::interrupts::disable();
    let ok = unsafe { configure_mouse() };
    x86_64::instructions::interrupts::enable();
    if ok {
        crate::serial::_print(format_args!("[mouse] PS/2 mouse initialised\n"));
    } else {
        crate::serial::_print(format_args!(
            "[mouse] WARN: PS/2 mouse not responding (continuing without it)\n"
        ));
    }
}

/// Run the PS/2 mouse configuration sequence. Returns `false` (without
/// aborting) when the device does not answer, so boot never hangs.
unsafe fn configure_mouse() -> bool {
    let mut cmd: Port<u8> = Port::new(0x64);

    // 1. Enable the auxiliary (mouse) port on the controller.
    send_controller_cmd(0xA8);

    // 2. Read the current controller configuration byte.
    send_controller_cmd(0x20);
    if !wait_read() {
        return false;
    }
    let mut data: Port<u8> = Port::new(0x60);
    let mut config = data.read();

    // 3. Modify config: enable IRQ12 (bit 1), enable aux clock (bit 5),
    //    disable translation (clear bit 6) — we want raw mouse packets.
    config |= 0x02 | 0x20;
    config &= !0x40;

    // 4. Write back the modified config.
    send_controller_cmd(0x60);
    wait_write();
    let mut data: Port<u8> = Port::new(0x60);
    data.write(config);

    // 5. Reset the mouse and wait for its self-test result.
    if !send_mouse_cmd(0xFF) {
        return false;
    }

    // 6. Set sample rate to 60 samples/sec.
    if !send_mouse_cmd(0xF3) || !send_mouse_cmd(60) {
        return false;
    }

    // 7. Enable packet streaming — mouse will start sending IRQ12.
    if !send_mouse_cmd(0xF4) {
        return false;
    }
    // Drain any residual byte (e.g. the 0xAA self-test answer) so the
    // interrupt handlers don't mistake it for a packet later.
    let s = cmd.read();
    if s & 0x01 != 0 {
        let mut data: Port<u8> = Port::new(0x60);
        let _ = data.read();
    }
    true
}
