//! Permission model — minimal privilege separation.
//!
//! Distinguishes between kernel privilege (Ring 0) and user privilege
//! (Ring 3). Provides basic resource access permission checks.
//!
//! ## Model
//!
//! - **Kernel** (Ring 0): full access to all hardware, memory, syscalls.
//! - **System processes** (Ring 3, uid=0): WM, Flutter Shell, sysutils.
//!   Can access framebuffer, input, filesystem.
//! - **User processes** (Ring 3, uid>0): application processes.
//!   Restricted framebuffer (only via WM surface), restricted filesystem.
//!
//! ## Not implemented (deferred)
//! - Per-file permissions (owner/group/other, rwx bits)
//! - User/groups database (/etc/passwd, /etc/group)
//! - setuid/setgid
//! - Capability-based security
//!
//! [MANUAL] Full permission enforcement requires:
//! - Ring3 page table isolation (user can't access kernel memory)
//! - User-mode syscall validation (check caller's privilege level)
//! - Per-process credential (uid, gid, capabilities)

use crate::process::{ProcessState, PROCESS_TABLE};

/// User ID: 0 = root/system, >0 = regular user.
pub type Uid = u32;

/// Privilege level for a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PrivilegeLevel {
    /// Kernel mode (Ring 0): full access.
    Kernel = 0,
    /// System process (Ring 3, uid=0): system services.
    System = 1,
    /// User process (Ring 3, uid>0): applications.
    User = 2,
}

/// Resource types for permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// Physical framebuffer (direct mmap access).
    Framebuffer,
    /// Input events (poll_input syscall).
    InputEvents,
    /// Filesystem read access.
    FileRead,
    /// Filesystem write access.
    FileWrite,
    /// Process management (kill, signal).
    ProcessControl,
    /// Exec (launch new processes).
    Exec,
}

/// Check if a process has permission to access a resource.
///
/// Returns true if access is granted, false if denied.
pub fn check_permission(pid: u32, resource: Resource) -> bool {
    // Kernel (pid=0) always has full access.
    if pid == 0 {
        return true;
    }

    let level = get_privilege_level(pid);

    match (level, resource) {
        // System processes: can access framebuffer, input, files, exec.
        (PrivilegeLevel::System, Resource::Framebuffer) => true,
        (PrivilegeLevel::System, Resource::InputEvents) => true,
        (PrivilegeLevel::System, Resource::FileRead) => true,
        (PrivilegeLevel::System, Resource::FileWrite) => true,
        (PrivilegeLevel::System, Resource::Exec) => true,
        (PrivilegeLevel::System, Resource::ProcessControl) => true,

        // User processes: restricted.
        // Can't access framebuffer directly — must go through WM.
        (PrivilegeLevel::User, Resource::Framebuffer) => false,
        // Can access input events via WM (WM routes to them).
        (PrivilegeLevel::User, Resource::InputEvents) => false,
        // Can read files.
        (PrivilegeLevel::User, Resource::FileRead) => true,
        // Can write to own files only (simplified: allow all for now).
        (PrivilegeLevel::User, Resource::FileWrite) => true,
        // Can exec new processes.
        (PrivilegeLevel::User, Resource::Exec) => true,
        // Can't kill other processes (only own).
        (PrivilegeLevel::User, Resource::ProcessControl) => false,

        // Unknown level: deny.
        _ => false,
    }
}

/// Get the privilege level of a process.
pub fn get_privilege_level(pid: u32) -> PrivilegeLevel {
    if pid == 0 {
        return PrivilegeLevel::Kernel;
    }

    let table = PROCESS_TABLE.lock();
    if let Some(proc) = table.get(pid) {
        if proc.state == ProcessState::Free {
            return PrivilegeLevel::Kernel;
        }
        // System processes have uid=0 (parent_pid=0 from boot_service).
        if proc.parent_pid == 0 {
            return PrivilegeLevel::System;
        }
        return PrivilegeLevel::User;
    }

    PrivilegeLevel::Kernel
}
