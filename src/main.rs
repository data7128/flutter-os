//! `aeros-os` binary: copies the bootable BIOS disk image produced by
//! `build.rs` to the workspace `target/` directory and prints the QEMU
//! command to run it.
//!
//! The image path is injected at compile time via the `BIOS_PATH` env var.

use std::path::PathBuf;

fn main() {
    let bios_path = env!("BIOS_PATH");
    let bios_path = PathBuf::from(bios_path);

    // Copy the image to a stable, easy-to-find location.
    let dest = PathBuf::from("target/aeros-os-bios.img");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::copy(&bios_path, &dest).ok();

    println!();
    println!("═══════════════════════════════════════════════");
    println!("  AeroOS BIOS disk image ready");
    println!("  Image: {}", dest.display());
    println!("  Size:  {} bytes", std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0));
    println!();
    println!("  Run in QEMU:");
    println!("    qemu-system-x86_64 \\");
    println!("      -drive format=raw,file={} \\", dest.display());
    println!("      -serial stdio");
    println!("═══════════════════════════════════════════════");
}
