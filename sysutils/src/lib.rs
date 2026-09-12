//! AeroOS System Utilities library — syscall wrappers and commands.
//!
//! The `sysutils` binary is a busybox-style multi-call utility: ls, cat,
//! ps, kill. It runs in Ring3 with real `int 0x80` syscalls.

#![no_std]
#![allow(dead_code)]

pub mod syscalls;
pub mod commands;
