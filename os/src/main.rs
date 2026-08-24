//! AeroOS bootable image entry point.
//!
//! This crate links the `bootloader` (v0.11) with the `aeros-kernel` binary.
//! Building it (`cargo build`) produces a standalone bootable disk image
//! that can be run in QEMU or written to a USB stick.

#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

bootloader::main_entry!(aeros_kernel::kernel_main);
