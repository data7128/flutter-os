//! Boot service — launches user-mode processes after kernel init.
//!
//! After all kernel subsystems are initialised, the boot service:
//! 1. Launches the window manager (WM) process
//! 2. Launches the Flutter Shell system desktop process
//!
//! [MANUAL] Both processes run in Ring3 user-mode. The actual
//! process launch requires the exec syscall + Ring3 context switch,
//! which is 【必须人工开发，AI无法完整生成】.
//!
//! ## Status
//! - Process scheduling logic: skeleton ✅
//! - Ring3 exec: 【需要人工调试】
//! - WM + Shell launch: 【暂未实现 — 等待 Ring3】

use crate::process;

/// Launch the system boot sequence: WM → Flutter Shell.
///
/// Called from `kernel_main()` after all kernel subsystems are up.
///
/// [MANUAL] The real implementation will:
/// 1. `exec("/sys/wm", &[])` — start the window manager
/// 2. Wait for WM to signal ready (via pipe or shared flag)
/// 3. `exec("/sys/flutter_shell", &[])` — start the desktop
///
/// For now, this just allocates process slots and logs.
pub fn launch_system() {
    // 1. Launch Window Manager.
    let wm_pid = process::PROCESS_TABLE.lock().alloc(0, b"wm");
    if wm_pid > 0 {
        crate::serial::_print(format_args!(
            "[boot_service] WM process allocated: pid={}\n", wm_pid
        ));
    } else {
        crate::serial::_print(format_args!(
            "[boot_service] ERROR: no process slot for WM\n"
        ));
    }

    // 2. Launch Flutter Shell.
    let shell_pid = process::PROCESS_TABLE.lock().alloc(0, b"flutter_shell");
    if shell_pid > 0 {
        crate::serial::_print(format_args!(
            "[boot_service] Flutter Shell process allocated: pid={}\n", shell_pid
        ));
    } else {
        crate::serial::_print(format_args!(
            "[boot_service] ERROR: no process slot for Flutter Shell\n"
        ));
    }

    // [MANUAL] exec the actual ELF binaries here:
    // crate::exec::sys_exec(b"/sys/wm\0", &[]);
    // → wait for WM ready
    // crate::exec::sys_exec(b"/sys/flutter_shell\0", &[]);

    crate::serial::_print(format_args!(
        "[boot_service] system processes scheduled\n"
    ));
}
