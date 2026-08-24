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
