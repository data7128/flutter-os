//! devfs — device filesystem mounted at /dev.
//!
//! Exposes kernel devices as VFS files:
//! - `/dev/hda`  — raw ATA disk (block device; offset = LBA × 512)
//! - `/dev/kbd`  — PS/2 keyboard scancode stream (read drains buffer)
//! - `/dev/serial` — serial port output
//! - `/dev/framebuffer` — framebuffer memory
//!
//! The device tree is static: inode 1 is the /dev directory, and each
//! device has a fixed inode id.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use super::{DirEntry, FileOps, FileType, VFS};

/// Fixed inode ids.
const DIR_INODE: u64 = 1;
const HDA_INODE: u64 = 2;
const KBD_INODE: u64 = 3;
const SERIAL_INODE: u64 = 4;
const FB_INODE: u64 = 5;

enum DevKind {
    BlockDisk,
    Keyboard,
    Serial,
    Framebuffer,
}

pub struct Devfs {
    inner: Mutex<Vec<(u64, &'static str, DevKind)>>,
}

impl Devfs {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(vec![
                (HDA_INODE, "hda", DevKind::BlockDisk),
                (KBD_INODE, "kbd", DevKind::Keyboard),
                (SERIAL_INODE, "serial", DevKind::Serial),
                (FB_INODE, "framebuffer", DevKind::Framebuffer),
            ]),
        }
    }
}

impl FileOps for Devfs {
    fn read(&self, inode_id: u64, offset: u64, buf: &mut [u8]) -> Result<usize, &'static str> {
        let devs = self.inner.lock();
        let (_, _, kind) = devs
            .iter()
            .find(|(id, _, _)| *id == inode_id)
            .ok_or("device not found")?;
        match kind {
            DevKind::BlockDisk => {
                // Block read: offset is treated as byte offset; read whole
                // sectors covering the requested range.
                let mut read_total = 0usize;
                let mut pos = offset;
                while read_total < buf.len() {
                    let lba = pos / 512;
                    let in_sector = (pos % 512) as usize;
                    let mut sector = [0u8; 512];
                    if !crate::ata::read_sector(lba, &mut sector) {
                        break;
                    }
                    let take = (512 - in_sector).min(buf.len() - read_total);
                    buf[read_total..read_total + take].copy_from_slice(&sector[in_sector..in_sector + take]);
                    pos += take as u64;
                    read_total += take;
                }
                Ok(read_total)
            }
            DevKind::Keyboard => {
                // Drain the scancode buffer.
                let mut n = 0usize;
                while n < buf.len() {
                    if let Some(sc) = crate::interrupts::SCANCODE_BUFFER.lock().pop() {
                        buf[n] = sc;
                        n += 1;
                    } else {
                        break;
                    }
                }
                Ok(n)
            }
            DevKind::Serial => {
                // Serial has no input side; nothing to read.
                Ok(0)
            }
            DevKind::Framebuffer => {
                // Read from the framebuffer, if mapped.
                let fb = crate::graphics::framebuffer_info();
                if let Some((base, len)) = fb {
                    let start = (offset as usize).min(len);
                    let end = (offset as usize + buf.len()).min(len);
                    let n = end - start;
                    if n > 0 {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                (base as *const u8).add(start),
                                buf.as_mut_ptr(),
                                n,
                            );
                        }
                    }
                    Ok(n)
                } else {
                    Ok(0)
                }
            }
        }
    }

    fn write(&self, inode_id: u64, offset: u64, buf: &[u8]) -> Result<usize, &'static str> {
        let devs = self.inner.lock();
        let (_, _, kind) = devs
            .iter()
            .find(|(id, _, _)| *id == inode_id)
            .ok_or("device not found")?;
        match kind {
            DevKind::BlockDisk => {
                // Sector-aligned writes only for simplicity.
                if offset % 512 != 0 {
                    return Err("unaligned block write");
                }
                let lba = offset / 512;
                if crate::ata::write_sectors(lba, buf) {
                    Ok(buf.len())
                } else {
                    Err("ata write failed")
                }
            }
            DevKind::Serial => {
                let s = core::str::from_utf8(buf).unwrap_or("");
                crate::serial::_print(format_args!("{}", s));
                Ok(buf.len())
            }
            DevKind::Keyboard => Err("not writable"),
            DevKind::Framebuffer => {
                let fb = crate::graphics::framebuffer_info();
                if let Some((base, len)) = fb {
                    let start = (offset as usize).min(len);
                    let n = (len - start).min(buf.len());
                    if n > 0 {
                        unsafe {
                            core::ptr::copy_nonoverlapping(buf.as_ptr(), (base as *mut u8).add(start), n);
                        }
                    }
                    Ok(n)
                } else {
                    Ok(0)
                }
            }
        }
    }

    fn size(&self, inode_id: u64) -> u64 {
        match inode_id {
            HDA_INODE => crate::ata::disk_info().total_sectors * 512,
            FB_INODE => crate::graphics::framebuffer_info().map(|(_, len)| len as u64).unwrap_or(0),
            _ => 0,
        }
    }

    fn file_type(&self, inode_id: u64) -> FileType {
        if inode_id == DIR_INODE {
            FileType::Directory
        } else {
            FileType::CharDevice
        }
    }

    fn readdir(&self, _inode_id: u64) -> Result<Vec<DirEntry>, &'static str> {
        let devs = self.inner.lock();
        Ok(devs
            .iter()
            .map(|(id, name, kind)| DirEntry {
                name: String::from(*name),
                inode_id: *id,
                file_type: match kind {
                    DevKind::BlockDisk => FileType::BlockDevice,
                    _ => FileType::CharDevice,
                },
            })
            .collect())
    }

    fn lookup(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        if dir_inode_id != DIR_INODE {
            return Err("not a directory");
        }
        let devs = self.inner.lock();
        devs.iter()
            .find(|(_, n, _)| *n == name)
            .map(|(id, _, _)| *id)
            .ok_or("device not found")
    }

    fn create(&self, _dir_inode_id: u64, _name: &str) -> Result<u64, &'static str> {
        Err("cannot create devices")
    }

    fn mkdir(&self, _dir_inode_id: u64, _name: &str) -> Result<u64, &'static str> {
        Err("cannot create directories in devfs")
    }

    fn truncate(&self, _inode_id: u64, _size: u64) -> Result<(), &'static str> {
        Err("cannot truncate devices")
    }

    fn unlink(&self, _dir_inode_id: u64, _name: &str) -> Result<(), &'static str> {
        Err("cannot remove devices")
    }
}

/// Initialise devfs and mount it at /dev.
pub fn init() {
    let devfs = Devfs::new();
    let boxed: Box<dyn FileOps> = Box::new(devfs);
    VFS.lock().mount("/dev", DIR_INODE, boxed);
    crate::serial::_print(format_args!("[devfs] mounted at /dev\n"));
}
