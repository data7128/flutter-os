//! Time subsystem — kernel-maintained system clock.
//!
//! The 8259 PIC timer (IRQ0) fires at a rate determined by the PIT
//! divisor. With the default 8259 PIC configuration, the PIT runs at
//! ~1193182 Hz / divisor. A divisor of 1193 gives ~1000 Hz (1 ms/tick).
//!
//! The timer interrupt handler increments a global `TICKS` counter.
//! `clock_gettime(CLOCK_MONOTONIC)` converts ticks to seconds + nanoseconds.
//!
//! ← FUTURE: implement RTC (CMOS) reading for `CLOCK_REALTIME`, and
//! HPET for higher precision timing.

use core::sync::atomic::{AtomicU64, Ordering};

/// PIT clock frequency (approximately 1193182 Hz).
const PIT_FREQUENCY: u64 = 1193182;

/// PIT divisor for ~1000 Hz (1 ms per tick).
const PIT_DIVISOR: u64 = 1193;

/// Ticks per second (approximate).
pub const TICKS_PER_SEC: u64 = PIT_FREQUENCY / PIT_DIVISOR; // ~1000

/// Nanoseconds per tick.
pub const NANOS_PER_TICK: u64 = 1_000_000_000 / TICKS_PER_SEC; // ~1_000_000 (1ms)

/// Global tick counter, incremented by the timer IRQ handler.
/// Uses atomic operations so it can be read safely from syscall context.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Called from `timer_interrupt_handler` in `idt.rs`.
/// Must be called with interrupts disabled (we're in an IRQ handler).
pub fn on_timer_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Get the current tick count.
pub fn tick_count() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Get system time as (seconds, nanoseconds) for CLOCK_MONOTONIC.
pub fn system_time() -> (u64, u64) {
    let ticks = tick_count();
    let secs = ticks / TICKS_PER_SEC;
    let sub_ticks = ticks % TICKS_PER_SEC;
    let nanos = sub_ticks * NANOS_PER_TICK;
    (secs, nanos)
}

/// Initialise the time subsystem by programming the PIT divisor.
///
/// Sets the PIT channel 0 to mode 2 (rate generator) with a divisor
/// of 1193, giving approximately 1000 Hz (1 ms/tick).
pub fn init() {
    // SAFETY: We're programming the PIT (8253/8254) timer chip.
    // Channel 0 is connected to IRQ0 of the 8259 PIC.
    unsafe {
        // I/O port 0x43 = PIT Mode/Command register
        // 0x34 = channel 0, lobyte/hibyte, mode 2 (rate generator), binary
        x86_64::instructions::port::Port::new(0x43).write(0x36u8);

        // Divisor = 1193 → ~1000 Hz
        let divisor: u16 = 1193;
        let mut channel0 = x86_64::instructions::port::Port::new(0x40);
        // Write low byte then high byte
        channel0.write((divisor & 0xFF) as u8);
        channel0.write((divisor >> 8) as u8);
    }

    crate::serial::_print(format_args!(
        "[time] PIT configured: {} Hz (divisor={}, {}ns/tick)\n",
        TICKS_PER_SEC, PIT_DIVISOR, NANOS_PER_TICK
    ));
}

/// A simple `Timespec` struct compatible with POSIX `struct timespec`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

impl Timespec {
    pub fn from_ticks() -> Self {
        let (secs, nanos) = system_time();
        Self {
            tv_sec: secs as i64,
            tv_nsec: nanos as i64,
        }
    }
}
