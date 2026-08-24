//! Build script: creates a bootable BIOS disk image from the kernel binary.
//!
//! The kernel ELF is provided by cargo's artifact-dependency feature as an
//! environment variable. We feed it to `bootloader::BiosBoot` which produces
//! a raw disk image that boots on legacy BIOS (and in QEMU).

use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());

    // Locate the kernel binary path set by cargo's artifact-dependency feature.
    let kernel = std::env::vars()
        .find(|(k, _)| k.starts_with("CARGO_BIN_FILE_"))
        .map(|(_, v)| PathBuf::from(v))
        .expect("build.rs: no CARGO_BIN_FILE_* env var found — is bindeps enabled?");

    eprintln!("build.rs: kernel binary = {}", kernel.display());

    // Create a bootable BIOS disk image.
    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel)
        .create_disk_image(&bios_path)
        .expect("build.rs: failed to create BIOS disk image");

    eprintln!("build.rs: BIOS image = {}", bios_path.display());

    // Pass the image path to src/main.rs via a compile-time env var.
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
    println!("cargo:rerun-if-changed=kernel/src");
}
