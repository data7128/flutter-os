//! Build script: creates a bootable BIOS disk image from the kernel binary,
//! then appends a formatted FAT32 partition containing `/bin/hello` (the
//! same minimal ELF user program the kernel's initramfs embeds).
//!
//! The kernel ELF is provided by cargo's artifact-dependency feature as an
//! environment variable. `bootloader::BiosBoot` produces the bootable part;
//! we then extend the image with a FAT32 filesystem so the kernel's ATA +
//! FAT32 drivers have a real disk to mount at `/disk`.

use std::path::PathBuf;

/// FAT32 partition geometry (8 MiB, 1 sector per cluster).
const FAT32_SECTORS: u32 = 16384; // 8 MiB / 512
const FAT32_RSVD_SEC_CNT: u16 = 32;
const FAT32_NUM_FATS: u8 = 2;
const FAT32_ROOT_CLUSTER: u32 = 2;
/// Sectors per FAT (computed: 16100 data clusters → 126 sectors).
const FAT32_FAT_SZ: u32 = 126;
/// First data cluster LBA (relative to partition start).
const FAT32_DATA_START: u32 = 32 + 2 * 126; // = 284

/// The minimal ELF64 user program written to the FAT32 root directory.
/// MUST stay in sync with `kernel/src/fs/initramfs.rs::HELLO_ELF`.
const HELLO_ELF: &[u8] = &[
    0x7f, 0x45, 0x4c, 0x46, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00, 0x78, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x38, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xb9, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb9, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x48, 0xc7, 0xc0, 0x03, 0x00, 0x00, 0x00, // mov rax, 3 (aeros write)
    0x48, 0xc7, 0xc7, 0x01, 0x00, 0x00, 0x00, // mov rdi, 1 (stdout)
    0x48, 0x8d, 0x35, 0x15, 0x00, 0x00, 0x00, // lea rsi, [rip+0x15]
    0x48, 0xc7, 0xc2, 0x16, 0x00, 0x00, 0x00, // mov rdx, 22
    0xcd, 0x80, // int 0x80
    0x48, 0xc7, 0xc0, 0x0b, 0x00, 0x00, 0x00, // mov rax, 11 (aeros exit)
    0x48, 0x31, 0xff, // xor rdi, rdi
    0xcd, 0x80, // int 0x80
    b'H', b'e', b'l', b'l', b'o', b' ', b'f', b'r', b'o', b'm', b' ',
    b'/', b'b', b'i', b'n', b'/', b'h', b'e', b'l', b'l', b'o', b'!', b'\n',
];

/// Build a 512-byte FAT32 boot sector (BPB) for the partition.
fn make_bpb() -> [u8; 512] {
    let mut bpb = [0u8; 512];
    // Jump instruction + OEM name.
    bpb[0] = 0xEB;
    bpb[1] = 0x3C;
    bpb[2] = 0x90;
    bpb[3..11].copy_from_slice(b"AEROS   ");
    // BPB.
    bpb[11..13].copy_from_slice(&512u16.to_le_bytes()); // BytsPerSec
    bpb[13] = 1; // SecPerClus
    bpb[14..16].copy_from_slice(&FAT32_RSVD_SEC_CNT.to_le_bytes());
    bpb[16] = FAT32_NUM_FATS;
    bpb[17..19].copy_from_slice(&0u16.to_le_bytes()); // RootEntCnt
    bpb[19..21].copy_from_slice(&0u16.to_le_bytes()); // TotSec16
    bpb[21] = 0xF8; // Media
    bpb[22..24].copy_from_slice(&0u16.to_le_bytes()); // FATSz16
    bpb[24..26].copy_from_slice(&63u16.to_le_bytes()); // SecPerTrk
    bpb[26..28].copy_from_slice(&255u16.to_le_bytes()); // NumHeads
    bpb[28..32].copy_from_slice(&0u32.to_le_bytes()); // HiddSec
    bpb[32..36].copy_from_slice(&FAT32_SECTORS.to_le_bytes()); // TotSec32
    bpb[36..40].copy_from_slice(&FAT32_FAT_SZ.to_le_bytes()); // FATSz32
    bpb[40..42].copy_from_slice(&0u16.to_le_bytes()); // ExtFlags
    bpb[42..44].copy_from_slice(&0u16.to_le_bytes()); // FSVer
    bpb[44..48].copy_from_slice(&FAT32_ROOT_CLUSTER.to_le_bytes()); // RootClus
    bpb[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo
    bpb[50..52].copy_from_slice(&6u16.to_le_bytes()); // BkBootSec
    bpb[64] = 0x80; // BS_DrvNum
    bpb[66] = 0x29; // BS_BootSig
    bpb[67..71].copy_from_slice(&0x1234_5678u32.to_le_bytes()); // BS_VolID
    bpb[71..82].copy_from_slice(b"AEROS DISK "); // BS_VolLab
    bpb[82..90].copy_from_slice(b"FAT32   "); // BS_FilSysType
    bpb[510] = 0x55;
    bpb[511] = 0xAA;
    bpb
}

/// Build the FAT table (cluster count = data sectors + 2).
fn make_fat() -> Vec<u8> {
    let data_clusters = FAT32_SECTORS - FAT32_RSVD_SEC_CNT as u32 - 2 * FAT32_FAT_SZ;
    let total_entries = (data_clusters + 2) as usize;
    let mut fat = vec![0u8; FAT32_FAT_SZ as usize * 512];
    let set = |fat: &mut [u8], idx: usize, v: u32| {
        let o = idx * 4;
        fat[o..o + 4].copy_from_slice(&v.to_le_bytes());
    };
    set(&mut fat, 0, 0x0FFF_FFF8);
    set(&mut fat, 1, 0x0FFF_FFFF);
    set(&mut fat, 2, 0x0FFF_FFFF); // root directory: EOC
    set(&mut fat, 3, 0x0FFF_FFFF); // hello file: EOC
    let _ = total_entries;
    fat
}

/// Build the root directory cluster (cluster 2): volume label + HELLO file.
fn make_root_cluster() -> Vec<u8> {
    let mut cluster = vec![0u8; 512];

    // Volume label entry.
    cluster[0..11].copy_from_slice(b"AEROS DISK ");
    cluster[11] = 0x08; // ATTR_VOLUME_ID
    cluster[13] = 0x10;
    cluster[14..16].copy_from_slice(&0x8A17u16.to_le_bytes());
    cluster[16..18].copy_from_slice(&0x4A4Bu16.to_le_bytes());

    // HELLO file entry (slot 1).
    let e = 32usize;
    cluster[e..e + 11].copy_from_slice(b"HELLO      "); // 8.3: "HELLO" + 6 spaces
    cluster[e + 11] = 0x20; // ATTR_ARCHIVE
    cluster[e + 13] = 0x10;
    cluster[e + 14..e + 16].copy_from_slice(&0x8A17u16.to_le_bytes());
    cluster[e + 16..e + 18].copy_from_slice(&0x4A4Bu16.to_le_bytes());
    cluster[e + 18..e + 20].copy_from_slice(&0x4A4Bu16.to_le_bytes());
    // FstClusHI (offset 20) stays 0; cluster 3 fits in 16 bits.
    cluster[e + 22..e + 24].copy_from_slice(&0x8A17u16.to_le_bytes());
    cluster[e + 24..e + 26].copy_from_slice(&0x4A4Bu16.to_le_bytes());
    cluster[e + 26..e + 28].copy_from_slice(&3u16.to_le_bytes()); // FstClusLO
    cluster[e + 28..e + 32].copy_from_slice(&(HELLO_ELF.len() as u32).to_le_bytes()); // FileSize
    cluster
}

/// Build the whole FAT32 partition image (as a sector stream).
fn make_fat32_partition() -> Vec<u8> {
    let mut img = Vec::with_capacity(FAT32_SECTORS as usize * 512);
    // Sector 0: BPB.
    img.extend_from_slice(&make_bpb());
    // Sector 1: FSInfo (marker).
    let mut fsinfo = [0u8; 512];
    fsinfo[0..4].copy_from_slice(b"RRaA");
    fsinfo[510..512].copy_from_slice(&0xAA55u16.to_le_bytes());
    img.extend_from_slice(&fsinfo);
    // Sectors 2..31: reserved (zeroed).
    img.extend_from_slice(&vec![0u8; 30 * 512]);
    // FAT1 + FAT2.
    let fat = make_fat();
    img.extend_from_slice(&fat);
    img.extend_from_slice(&fat);
    // Data region: root cluster (2) + hello cluster (3) + rest zeroed.
    let mut root = make_root_cluster();
    // Write HELLO_ELF into cluster 3 (offset 512 in data region).
    let mut hello_cluster = vec![0u8; 512];
    hello_cluster[..HELLO_ELF.len()].copy_from_slice(HELLO_ELF);
    root.extend_from_slice(&hello_cluster);
    img.extend_from_slice(&root);
    // Fill the remaining data sectors with zeros.
    let data_sectors = FAT32_SECTORS as usize - FAT32_DATA_START as usize;
    let used = img.len() / 512;
    let remaining = data_sectors - (used - FAT32_DATA_START as usize);
    img.extend_from_slice(&vec![0u8; remaining * 512]);
    debug_assert_eq!(img.len(), FAT32_SECTORS as usize * 512);
    img
}

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

    // ── Append FAT32 partition ──────────────────────────────────────
    let mut image = std::fs::read(&bios_path).expect("build.rs: read bios.img");
    let boot_sectors = (image.len() / 512) as u32;
    if image.len() % 512 != 0 {
        // Pad to a sector boundary.
        let pad = 512 - (image.len() % 512);
        image.extend_from_slice(&vec![0u8; pad]);
    }
    eprintln!("build.rs: boot image = {} sectors", boot_sectors);

    // Build and append the FAT32 partition.
    let fat32 = make_fat32_partition();
    let fat32_start = boot_sectors;
    image.extend_from_slice(&fat32);

    // Update the MBR: add partition entry 3 (FAT32 LBA, 0x0C).
    if image[510] != 0x55 || image[511] != 0xAA {
        panic!("build.rs: boot image has no valid MBR signature");
    }
    let entry = 446 + 2 * 16; // third partition entry
    image[entry] = 0x00; // not bootable
    image[entry + 4] = 0x0C; // partition type FAT32 LBA
    image[entry + 8..entry + 12].copy_from_slice(&fat32_start.to_le_bytes());
    image[entry + 12..entry + 16].copy_from_slice(&FAT32_SECTORS.to_le_bytes());
    image[510] = 0x55;
    image[511] = 0xAA;

    std::fs::write(&bios_path, &image).expect("build.rs: write extended image");
    eprintln!(
        "build.rs: FAT32 partition appended @ LBA {} ({} sectors) — image total {} bytes",
        fat32_start,
        FAT32_SECTORS,
        image.len()
    );

    // Pass the image path to src/main.rs via a compile-time env var.
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
    println!("cargo:rerun-if-changed=kernel/src");
    println!("cargo:rerun-if-changed=build.rs");
}
