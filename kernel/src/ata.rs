//! ATA/IDE PIO disk driver — x86_64 28-bit LBA.
//!
//! Talks to the primary IDE channel via legacy I/O ports (0x1F0-0x1F7,
//! 0x3F6). QEMU's default `-drive format=raw,file=...` attaches the disk
//! as the primary master, so this is all we need for the emulated
//! environment (and for simple real hardware with PIO-capable IDE).
//!
//! Operations:
//! - `detect()` — probe the primary master, issue IDENTIFY, fill geometry
//! - `read_sectors(lba, buf)` / `write_sectors(lba, buf)` — 28-bit LBA PIO
//!
//! All ports are accessed through `x86_64::instructions::port::Port`.

use x86_64::instructions::port::Port;

/// I/O ports of the primary IDE channel.
const PORT_DATA: u16 = 0x1F0; // data (16-bit)
#[allow(dead_code)]
const PORT_ALT_STATUS: u16 = 0x3F6; // alt status / control
const PORT_ERR: u16 = 0x1F1; // error / features
const PORT_SEC_COUNT: u16 = 0x1F2; // sector count
const PORT_LBA_LO: u16 = 0x1F3; // LBA 0-7
const PORT_LBA_MID: u16 = 0x1F4; // LBA 8-15
const PORT_LBA_HI: u16 = 0x1F5; // LBA 16-23
const PORT_DRIVE: u16 = 0x1F6; // drive / LBA 24-27
const PORT_STATUS: u16 = 0x1F7; // status / command

/// Status register bits.
const STATUS_BSY: u8 = 0x80;
#[allow(dead_code)]
const STATUS_DRDY: u8 = 0x40;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

/// Commands.
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_IDENTIFY: u8 = 0xEC;

/// Disk geometry discovered at detect time.
#[derive(Debug, Clone, Copy)]
pub struct DiskInfo {
    /// Total number of 512-byte sectors (from IDENTIFY word 60-61).
    pub total_sectors: u64,
    /// Model string (from IDENTIFY words 27-46, trimmed).
    pub model: [u8; 40],
}

/// Global disk state.
static mut DISK_PRESENT: bool = false;
static mut DISK_INFO: DiskInfo = DiskInfo {
    total_sectors: 0,
    model: [0; 40],
};

/// Wait for the controller to be ready (BSY clear).
fn wait_not_busy() -> bool {
    let mut status = Port::<u8>::new(PORT_STATUS);
    for _ in 0..100_000 {
        let s = unsafe { status.read() };
        if s & STATUS_BSY == 0 {
            return true;
        }
    }
    false
}

/// Wait for DRQ (data request) after issuing a command.
fn wait_drq() -> bool {
    let mut status = Port::<u8>::new(PORT_STATUS);
    for _ in 0..100_000 {
        let s = unsafe { status.read() };
        if s & STATUS_BSY == 0 {
            if s & STATUS_DRQ != 0 {
                return true;
            }
            // Error without DRQ.
            if s & STATUS_ERR != 0 {
                return false;
            }
        }
    }
    false
}

/// Read one 512-byte sector via PIO. `lba` must be < 2^28.
pub fn read_sector(lba: u64, buf: &mut [u8; 512]) -> bool {
    read_sectors(lba, &mut buf[..])
}

/// Read `buf.len() / 512` consecutive sectors starting at `lba`.
/// Returns `false` on error/timeout.
pub fn read_sectors(lba: u64, buf: &mut [u8]) -> bool {
    if !disk_present() || lba >= (1 << 28) {
        return false;
    }
    let sectors = buf.len() / 512;
    if sectors == 0 || buf.len() % 512 != 0 {
        return false;
    }
    if sectors > 256 {
        // Split into chunks of ≤ 255 sectors.
        for chunk in 0..((sectors + 254) / 255) {
            let start = chunk * 255;
            let n = (sectors - start).min(255);
            if !read_sectors(lba + start as u64, &mut buf[start * 512..(start + n) * 512]) {
                return false;
            }
        }
        return true;
    }
    if !wait_not_busy() {
        return false;
    }

    // Program the controller: sector count + 28-bit LBA.
    unsafe {
        let mut sec_count = Port::<u8>::new(PORT_SEC_COUNT);
        let mut lba_lo = Port::<u8>::new(PORT_LBA_LO);
        let mut lba_mid = Port::<u8>::new(PORT_LBA_MID);
        let mut lba_hi = Port::<u8>::new(PORT_LBA_HI);
        let mut drive = Port::<u8>::new(PORT_DRIVE);
        let mut cmd = Port::<u8>::new(PORT_STATUS);

        sec_count.write((sectors & 0xFF) as u8);
        lba_lo.write((lba & 0xFF) as u8);
        lba_mid.write(((lba >> 8) & 0xFF) as u8);
        lba_hi.write(((lba >> 16) & 0xFF) as u8);
        drive.write(0xE0 | (((lba >> 24) & 0x0F) as u8)); // master + LBA
        cmd.write(CMD_READ_SECTORS);

        // Read each sector.
        let mut data = Port::<u16>::new(PORT_DATA);
        for s in 0..sectors {
            if !wait_drq() {
                return false;
            }
            let base = s * 512;
            for i in 0..256 {
                let w = data.read();
                buf[base + i * 2] = (w & 0xFF) as u8;
                buf[base + i * 2 + 1] = (w >> 8) as u8;
            }
        }
    }
    true
}

/// Write `buf.len() / 512` consecutive sectors starting at `lba`.
/// Returns `false` on error/timeout.
pub fn write_sectors(lba: u64, buf: &[u8]) -> bool {
    if !disk_present() || lba >= (1 << 28) {
        return false;
    }
    let sectors = buf.len() / 512;
    if sectors == 0 || buf.len() % 512 != 0 {
        return false;
    }
    if sectors > 256 {
        for chunk in 0..((sectors + 254) / 255) {
            let start = chunk * 255;
            let n = (sectors - start).min(255);
            if !write_sectors(lba + start as u64, &buf[start * 512..(start + n) * 512]) {
                return false;
            }
        }
        return true;
    }
    if !wait_not_busy() {
        return false;
    }

    unsafe {
        let mut sec_count = Port::<u8>::new(PORT_SEC_COUNT);
        let mut lba_lo = Port::<u8>::new(PORT_LBA_LO);
        let mut lba_mid = Port::<u8>::new(PORT_LBA_MID);
        let mut lba_hi = Port::<u8>::new(PORT_LBA_HI);
        let mut drive = Port::<u8>::new(PORT_DRIVE);
        let mut cmd = Port::<u8>::new(PORT_STATUS);

        sec_count.write((sectors & 0xFF) as u8);
        lba_lo.write((lba & 0xFF) as u8);
        lba_mid.write(((lba >> 8) & 0xFF) as u8);
        lba_hi.write(((lba >> 16) & 0xFF) as u8);
        drive.write(0xE0 | (((lba >> 24) & 0x0F) as u8)); // master + LBA
        cmd.write(CMD_WRITE_SECTORS);

        let mut data = Port::<u16>::new(PORT_DATA);
        for s in 0..sectors {
            if !wait_drq() {
                return false;
            }
            let base = s * 512;
            for i in 0..256 {
                let lo = buf[base + i * 2] as u16;
                let hi = buf[base + i * 2 + 1] as u16;
                data.write(lo | (hi << 8));
            }
        }
        // Flush cache (send FLUSH CACHE command) so writes are durable.
        if !wait_not_busy() {
            return false;
        }
        cmd.write(0xE7); // CMD_FLUSH_CACHE
        wait_not_busy();
    }
    true
}

/// Probe the primary master and (if present) read IDENTIFY data.
pub fn detect() -> bool {
    unsafe {
        // Check that the controller exists: status port reads back != 0xFF.
        let mut status = Port::<u8>::new(PORT_STATUS);
        let s = status.read();
        if s == 0xFF {
            // No controller on this channel.
            return false;
        }
        // Reset / select master and issue IDENTIFY.
        let mut drive = Port::<u8>::new(PORT_DRIVE);
        drive.write(0xA0); // select master
        wait_not_busy();
        let mut cmd = Port::<u8>::new(PORT_STATUS);
        cmd.write(CMD_IDENTIFY);
        if !wait_not_busy() {
            return false;
        }
        // If ERR is set with no drive, there's no device.
        let mut err = Port::<u8>::new(PORT_ERR);
        let e = err.read();
        if e != 0 {
            // No drive present.
            return false;
        }
        // Read 256 words of IDENTIFY data.
        let mut data = Port::<u16>::new(PORT_DATA);
        let mut ident = [0u16; 256];
        if !wait_drq() {
            return false;
        }
        for i in 0..256 {
            ident[i] = data.read();
        }
        // Word 0 bit 0 (ATAPI/response flags) is not a reliable presence
        // probe — real devices (incl. QEMU) return 0x40 there. Presence is
        // already established: no ERR, DRQ asserted, and we read 256 words
        // back. Just sanity-check the LBA support bit instead.
        if ident[49] & (1 << 9) == 0 {
            // LBA supported bit missing — treat as "not a disk we handle".
            return false;
        }
        // Total sectors: words 60-61 (28-bit LBA).
        let total = (ident[61] as u64) << 16 | ident[60] as u64;
        // Model string: words 27-46, big-endian pairs.
        let mut model = [0u8; 40];
        for w in 0..20 {
            let word = ident[27 + w];
            model[w * 2] = (word >> 8) as u8;
            model[w * 2 + 1] = (word & 0xFF) as u8;
        }
        // Trim trailing spaces.
        let mut end = model.len();
        while end > 0 && model[end - 1] == b' ' || end > 0 && model[end - 1] == 0 {
            end -= 1;
        }
        for i in end..model.len() {
            model[i] = 0;
        }

        DISK_PRESENT = true;
        DISK_INFO = DiskInfo { total_sectors: total, model };
        true
    }
}

/// Whether a disk is present.
pub fn disk_present() -> bool {
    unsafe { DISK_PRESENT }
}

/// Return disk info.
pub fn disk_info() -> DiskInfo {
    unsafe { DISK_INFO }
}

/// Initialise the ATA driver.
pub fn init() {
    if detect() {
        let info = disk_info();
        crate::serial::_print(format_args!(
            "[ata] primary master detected: {} sectors, model \"{}\"\n",
            info.total_sectors,
            core::str::from_utf8(&info.model).unwrap_or("<bad model>")
        ));
    } else {
        crate::serial::_print(format_args!("[ata] no disk on primary master\n"));
    }
}
