//! Minimal implementations of compiler-builtins memory functions.
//!
//! Without `-Z build-std-features=compiler-builtins-mem`, the linker
//! expects `memcpy`, `memset`, `memmove`, and `memcmp` to be provided
//! by the crate itself. These are simple byte-wise implementations.

use core::ffi::{c_char, c_int, c_void};

/// Copy `n` bytes from `src` to `dest`. The regions must not overlap
/// (use `memmove` for overlapping regions).
#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest = dest as *mut u8;
    let src = src as *const u8;
    for i in 0..n {
        *dest.add(i) = *src.add(i);
    }
    dest as *mut c_void
}

/// Copy `n` bytes from `src` to `dest`, handling overlapping regions.
#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest = dest as *mut u8;
    let src = src as *const u8;
    if (dest as usize) < (src as usize) {
        for i in 0..n {
            *dest.add(i) = *src.add(i);
        }
    } else {
        for i in (0..n).rev() {
            *dest.add(i) = *src.add(i);
        }
    }
    dest as *mut c_void
}

/// Fill `n` bytes at `s` with byte value `c`.
#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let s = s as *mut u8;
    let byte = c as u8;
    for i in 0..n {
        *s.add(i) = byte;
    }
    s as *mut c_void
}

/// Compare `n` bytes at `s1` and `s2`. Returns 0 if equal, negative if
/// `s1 < s2`, positive if `s1 > s2` (at the first differing byte).
#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let s1 = s1 as *const u8;
    let s2 = s2 as *const u8;
    for i in 0..n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return (a as c_int) - (b as c_int);
        }
    }
    0
}

/// Length of a null-terminated string.
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let s = s as *const u8;
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}
