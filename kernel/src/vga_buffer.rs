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
