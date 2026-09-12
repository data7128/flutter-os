//! AeroOS System Utilities — ls, cat, ps, kill.
//!
//! A busybox-style multi-call binary: the first argument determines
//! which utility to run. Runs in Ring3 on AeroOS with real syscalls.
//!
//! When launched with no arguments (as the kernel does at boot), it
//! runs a self-check demo: lists `/`, then fork+execs `/bin/hello`
//! and waitpids the child.

#![no_std]
#![no_main]

use sysutils::commands;
use sysutils::syscalls;

/// Panic handler: print a message and exit.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    syscalls::print_str("sysutils: panic: ");
    if let Some(msg) = info.message().as_str() {
        syscalls::print_str(msg);
    }
    syscalls::println("");
    syscalls::exit(70);
}

/// True `_start` entry — reads argc/argv off the user stack and calls
/// `aero_main`, then exits with its return code.
core::arch::global_asm!(
    r#"
.global _start
_start:
    mov rdi, [rsp]        // argc
    lea rsi, [rsp + 8]    // argv
    call aero_main
    mov rdi, rax          // exit status
    mov rax, 11           // SYS_EXIT
    int 0x80
    hlt
"#
);

/// Rust entry point called by `_start`.
///
/// `argv` points at a null-terminated array of string pointers.
#[unsafe(no_mangle)]
pub extern "C" fn aero_main(argc: u64, argv: *const *const u8) -> i32 {
    // Collect argv into a fixed array of borrowed byte slices.
    let mut args: [&[u8]; 16] = [b""; 16];
    let mut n: usize = 0;
    if argc > 0 && !argv.is_null() {
        let mut i = 0usize;
        while i < argc as usize && n < 16 {
            let p = unsafe { *argv.add(i) };
            if p.is_null() {
                break;
            }
            let len = unsafe {
                let mut l = 0usize;
                while *p.add(l) != 0 {
                    l += 1;
                    if l > 255 {
                        break;
                    }
                }
                l
            };
            args[n] = unsafe { core::slice::from_raw_parts(p, len) };
            n += 1;
            i += 1;
        }
    }
    let argv_slice = &args[..n];

    if argv_slice.len() < 2 {
        // No command given (kernel boot): run the demo.
        return demo();
    }

    let cmd = argv_slice[1];
    if eq(cmd, b"ls") {
        commands::ls::run(argv_slice.get(2).copied().unwrap_or(b"/"))
    } else if eq(cmd, b"cat") {
        if argv_slice.len() < 3 {
            syscalls::println("cat: missing file path");
            1
        } else {
            commands::cat::run(argv_slice[2])
        }
    } else if eq(cmd, b"ps") {
        commands::ps::run()
    } else if eq(cmd, b"kill") {
        if argv_slice.len() < 3 {
            syscalls::println("kill: missing PID");
            1
        } else {
            commands::kill::run(argv_slice[2])
        }
    } else {
        syscalls::print_str("sysutils: unknown command: ");
        syscalls::print_str(core::str::from_utf8(cmd).unwrap_or("?"));
        syscalls::println("");
        1
    }
}

/// Boot demo (no argv): prove the user-mode stack works end to end —
/// list the root directory, then fork+exec `/bin/hello` and reap it.
fn demo() -> i32 {
    let pid = syscalls::getpid();
    syscalls::print_str("=== AeroOS sysutils in Ring3 (pid=");
    syscalls::print_u64(pid as u64);
    syscalls::println(") ===");

    syscalls::println("--- ls / ---");
    commands::ls::run(b"/");

    syscalls::println("--- fork + exec(/bin/hello) + waitpid ---");
    let r = syscalls::fork();
    if r == 0 {
        // Child: replace itself with /bin/hello.
        let e = syscalls::exec(b"/bin/hello", core::ptr::null());
        syscalls::print_str("exec failed: ");
        syscalls::print_i64(e);
        syscalls::println("");
        syscalls::exit(1);
    } else if r > 0 {
        let mut status: i32 = 0;
        let w = syscalls::waitpid(r, &mut status);
        syscalls::print_str("--- child ");
        syscalls::print_u64(r as u64);
        syscalls::print_str(" reaped, waitpid=");
        syscalls::print_i64(w);
        syscalls::print_str(", status=");
        syscalls::print_i64(status as i64);
        syscalls::println(" ---");
        0
    } else {
        syscalls::print_str("fork failed: ");
        syscalls::print_i64(r);
        syscalls::println("");
        1
    }
}

/// Compare two byte slices.
fn eq(a: &[u8], b: &[u8]) -> bool {
    a == b
}

// ---------------------------------------------------------------------------
// Compiler-rt shims: the Rust compiler lowers large zero-init / copies to
// memcpy/memset/memmove/memcmp calls. With no libc and a fully static link
// (and our linker script discarding .got/.rela), those calls would resolve to
// absolute address 8 and page-fault. Provide real implementations.
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0usize;
    while i < n {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
    dst
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dst as usize) < (src as usize) {
        let mut i = 0usize;
        while i < n {
            *dst.add(i) = *src.add(i);
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dst.add(i) = *src.add(i);
        }
    }
    dst
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0usize;
    while i < n {
        *s.add(i) = c as u8;
        i += 1;
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i < n {
        let x = *a.add(i);
        let y = *b.add(i);
        if x != y {
            return (x as i32) - (y as i32);
        }
        i += 1;
    }
    0
}
