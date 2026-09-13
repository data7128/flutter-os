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

/// Non-blocking read from COM1: returns the next byte if the receiver
/// holds data, or `None` if the FIFO is empty. Used as the stdin source
/// for Ring3 processes (`read(0, ...)`) so the shell can take commands
/// typed on the host terminal (`qemu -serial stdio`).
pub fn try_read_byte() -> Option<u8> {
    use x86_64::instructions::port::Port;
    // Line Status Register (COM1+5): bit 0 = Data Ready.
    let mut lsr = Port::<u8>::new(0x3FD);
    let status = unsafe { lsr.read() };
    if status & 0x01 != 0 {
        // Receiver Buffer Register (COM1+0).
        let mut rbr = Port::<u8>::new(0x3F8);
        Some(unsafe { rbr.read() })
    } else {
        None
    }
}
