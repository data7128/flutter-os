//! VGA text-mode buffer (0xB8000).
//!
//! A classic 80x25 color text buffer used for the early boot log before
//! the framebuffer shell is up. `_print` is shared with the serial console
//! through the `print!`/`println!` macros.
//!
//! We use raw volatile pointer operations (`core::ptr::write_volatile` /
//! `read_volatile`) instead of the `volatile` crate so that the static
//! initializer avoids creating a dangling reference at compile time
//! (nightly Rust's provenance check rejects `&mut *(0xb8000 as *mut _)`
//! in a `const` context).

use core::fmt::{self, Write};
use core::ptr;
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

/// The VGA text buffer lives at this fixed physical address.
const VGA_BUFFER_ADDR: usize = 0xb8000;

/// A view over the 25x80 screen, wrapped in a spinlock so interrupts can
/// also write safely. The buffer is a raw pointer to avoid creating a
/// dangling reference in const context.
pub static WRITER: Mutex<Writer> = Mutex::new(Writer {
    column_position: 0,
    row_position: 0,
    color_code: ColorCode::new(Color::LightGreen, Color::Black),
    buffer: VGA_BUFFER_ADDR as *mut ScreenChar,
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

/// Low-level entry point used by the `print!` macro. Acquires the writer
/// and forwards the formatted arguments.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    WRITER.lock().write_fmt(args).unwrap();
}

/// Clear the whole screen.
#[allow(dead_code)]
pub fn clear_screen() {
    for row in 0..BUFFER_HEIGHT {
        WRITER.lock().clear_row(row);
    }
}
