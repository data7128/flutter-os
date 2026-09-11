//! FAT32 filesystem driver — backed by the ATA disk driver.
//!
//! Implements the `FileOps` trait so a FAT32 partition can be mounted
//! into the VFS. Supports read/write/create/mkdir/truncate/unlink/readdir
//! on files and directories, including cluster-chain traversal and
//! allocation of new clusters for writes.
//!
//! Layout discovery:
//! - Scan the MBR partition table for a 0x0B/0x0C (FAT32) partition
//! - Parse the BPB to compute FAT location, data region and root cluster
//! - Cache the whole FAT1 table in memory for fast cluster lookups
//!
//! Notes:
//! - Short 8.3 names are fully supported; long-name (0x0F) entries are
//!   skipped when reading and not created when writing.
//! - The FAT cache is written back in full after mutations (simple but
//!   correct; optimisation possible later).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

use super::{DirEntry, FileOps, FileType, VFS};

/// MBR partition entry type for FAT32 LBA / FAT32.
const PART_TYPE_FAT32: u8 = 0x0C;
const PART_TYPE_FAT32_CHS: u8 = 0x0B;

/// FAT cluster values.
const FAT_EOC: u32 = 0x0FFF_FFFF; // end of chain
const FAT_FREE: u32 = 0x0000_0000;

/// Attribute bits in a directory entry.
const ATTR_DIR: u8 = 0x10;
const ATTR_LFN: u8 = 0x0F;
const ATTR_ARCHIVE: u8 = 0x20;

/// A FAT32 directory entry (32 bytes, little-endian).
#[repr(C, packed)]
struct RawDirEntry {
    name: [u8; 11],
    attr: u8,
    nt_res: u8,
    crt_time_tenth: u8,
    crt_time: u16,
    crt_date: u16,
    lst_acc_date: u16,
    fst_clus_hi: u16,
    wrt_time: u16,
    wrt_date: u16,
    fst_clus_lo: u16,
    file_size: u32,
}

impl RawDirEntry {
    fn is_free(&self) -> bool {
        self.name[0] == 0x00 || self.name[0] == 0xE5
    }

    fn is_lfn(&self) -> bool {
        self.attr & ATTR_LFN == ATTR_LFN
    }

    fn is_dir(&self) -> bool {
        self.attr & ATTR_DIR != 0
    }

    fn first_cluster(&self) -> u32 {
        ((self.fst_clus_hi as u32) << 16) | self.fst_clus_lo as u32
    }
}

/// Inode bookkeeping inside the FAT32 filesystem.
#[derive(Debug, Clone, Copy)]
struct FatInode {
    first_cluster: u32,
    size: u32,
    is_dir: bool,
}

struct Fat32Inner {
    /// Absolute LBA where this partition starts on the disk.
    part_start_lba: u64,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    rsvd_sec_cnt: u16,
    num_fats: u8,
    fat_sz32: u32,
    root_cluster: u32,
    /// Absolute LBA of the data region.
    data_start_lba: u64,
    /// Cached FAT1 table (index = cluster number).
    fat_cache: Vec<u32>,
    /// VFS inode table.
    inodes: BTreeMap<u64, FatInode>,
    next_inode: u64,
}

pub struct Fat32 {
    inner: Mutex<Fat32Inner>,
}

impl Fat32 {
    fn cluster_to_lba(&self, inner: &Fat32Inner, cluster: u32) -> u64 {
        inner.data_start_lba
            + ((cluster as u64 - 2) * inner.sectors_per_cluster as u64)
    }

    /// Read one cluster's data into `buf` (cluster-sized buffer).
    fn read_cluster(&self, inner: &Fat32Inner, cluster: u32, buf: &mut [u8]) -> bool {
        let lba = self.cluster_to_lba(inner, cluster);
        let bytes = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        if buf.len() < bytes {
            return false;
        }
        crate::ata::read_sectors(lba, &mut buf[..bytes])
    }

    /// Write one cluster's data from `buf`.
    fn write_cluster(&self, inner: &Fat32Inner, cluster: u32, buf: &[u8]) -> bool {
        let lba = self.cluster_to_lba(inner, cluster);
        let bytes = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        if buf.len() < bytes {
            return false;
        }
        crate::ata::write_sectors(lba, &buf[..bytes])
    }

    /// Write the FAT cache back to disk (all FAT copies).
    fn flush_fat(&self, inner: &Fat32Inner) -> bool {
        let fat_start = inner.part_start_lba + inner.rsvd_sec_cnt as u64;
        let fat_bytes = inner.fat_sz32 as usize * inner.bytes_per_sector as usize;
        let mut table = Vec::with_capacity(fat_bytes);
        for &v in &inner.fat_cache {
            table.extend_from_slice(&v.to_le_bytes());
        }
        table.resize(fat_bytes, 0);
        // Write FAT1.
        if !crate::ata::write_sectors(fat_start, &table) {
            return false;
        }
        // Write FAT2 copy.
        for i in 1..inner.num_fats {
            let start = fat_start + (i as u64) * inner.fat_sz32 as u64;
            if !crate::ata::write_sectors(start, &table) {
                return false;
            }
        }
        true
    }

    /// Load the FAT1 table into memory.
    fn load_fat(&self, inner: &mut Fat32Inner) -> bool {
        let fat_start = inner.part_start_lba + inner.rsvd_sec_cnt as u64;
        let fat_sectors = inner.fat_sz32 as usize;
        let mut table_bytes = vec![0u8; fat_sectors * inner.bytes_per_sector as usize];
        if !crate::ata::read_sectors(fat_start, &mut table_bytes) {
            return false;
        }
        let mut fat = Vec::with_capacity(fat_sectors * inner.bytes_per_sector as usize / 4);
        for i in 0..fat_sectors * inner.bytes_per_sector as usize / 4 {
            let o = i * 4;
            let v = u32::from_le_bytes([
                table_bytes[o],
                table_bytes[o + 1],
                table_bytes[o + 2],
                table_bytes[o + 3],
            ]);
            fat.push(v);
        }
        inner.fat_cache = fat;
        true
    }

    /// Collect the full cluster chain starting at `first`.
    fn cluster_chain(&self, inner: &Fat32Inner, first: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut c = first;
        // Guard against cycles / insane chains.
        for _ in 0..1_000_000 {
            if c < 2 || c as usize >= inner.fat_cache.len() {
                break;
            }
            chain.push(c);
            let next = inner.fat_cache[c as usize];
            if next >= FAT_EOC - 1 || next == 0 {
                break;
            }
            c = next;
        }
        chain
    }

    /// Allocate a free cluster and mark it EOC. Returns cluster or None.
    fn alloc_cluster(&self, inner: &mut Fat32Inner) -> Option<u32> {
        for i in 2..inner.fat_cache.len() {
            if inner.fat_cache[i] == FAT_FREE {
                inner.fat_cache[i] = FAT_EOC;
                return Some(i as u32);
            }
        }
        None
    }

    /// Free a cluster chain (mark all FAT entries as free).
    fn free_chain(&self, inner: &mut Fat32Inner, first: u32) {
        let chain = self.cluster_chain(inner, first);
        for c in chain {
            inner.fat_cache[c as usize] = FAT_FREE;
        }
    }

    /// Read directory entries from a directory cluster chain.
    fn read_dir_raw(&self, inner: &Fat32Inner, dir_cluster: u32) -> Vec<RawDirEntry> {
        let chain = self.cluster_chain(inner, dir_cluster);
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let mut entries = Vec::new();
        let mut buf = vec![0u8; bytes_per_cluster];
        for c in chain {
            if !self.read_cluster(inner, c, &mut buf) {
                break;
            }
            for i in 0..bytes_per_cluster / 32 {
                let off = i * 32;
                let mut raw = RawDirEntry {
                    name: [0; 11],
                    attr: 0,
                    nt_res: 0,
                    crt_time_tenth: 0,
                    crt_time: 0,
                    crt_date: 0,
                    lst_acc_date: 0,
                    fst_clus_hi: 0,
                    wrt_time: 0,
                    wrt_date: 0,
                    fst_clus_lo: 0,
                    file_size: 0,
                };
                // Copy field-by-field to avoid unaligned packed access UB.
                raw.name.copy_from_slice(&buf[off..off + 11]);
                raw.attr = buf[off + 11];
                raw.nt_res = buf[off + 12];
                raw.crt_time_tenth = buf[off + 13];
                raw.crt_time = u16::from_le_bytes([buf[off + 14], buf[off + 15]]);
                raw.crt_date = u16::from_le_bytes([buf[off + 16], buf[off + 17]]);
                raw.lst_acc_date = u16::from_le_bytes([buf[off + 18], buf[off + 19]]);
                raw.fst_clus_hi = u16::from_le_bytes([buf[off + 20], buf[off + 21]]);
                raw.wrt_time = u16::from_le_bytes([buf[off + 22], buf[off + 23]]);
                raw.wrt_date = u16::from_le_bytes([buf[off + 24], buf[off + 25]]);
                raw.fst_clus_lo = u16::from_le_bytes([buf[off + 26], buf[off + 27]]);
                raw.file_size = u32::from_le_bytes([
                    buf[off + 28],
                    buf[off + 29],
                    buf[off + 30],
                    buf[off + 31],
                ]);
                entries.push(raw);
            }
        }
        entries
    }

    /// Write a raw directory entry back to the first free slot in a
    /// directory's cluster chain, extending the directory if needed.
    fn write_dir_entry(
        &self,
        inner: &mut Fat32Inner,
        dir_cluster: u32,
        entry: &RawDirEntry,
        create_new: bool,
    ) -> Result<(), &'static str> {
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let chain = self.cluster_chain(inner, dir_cluster);
        let mut buf = vec![0u8; bytes_per_cluster];

        // First pass: look for a free slot (0xE5 or 0x00) in existing clusters.
        for c in &chain {
            if !self.read_cluster(inner, *c, &mut buf) {
                return Err("ata read failed");
            }
            for i in 0..bytes_per_cluster / 32 {
                let off = i * 32;
                if buf[off] == 0x00 || buf[off] == 0xE5 {
                    self.pack_entry(entry, &mut buf[off..off + 32]);
                    if !self.write_cluster(inner, *c, &buf) {
                        return Err("ata write failed");
                    }
                    return Ok(());
                }
            }
        }

        if !create_new {
            return Err("no free slot in directory");
        }

        // Extend the directory with a new cluster.
        let new_cluster = self.alloc_cluster(inner).ok_or("out of clusters")?;
        // Link the new cluster onto the chain: find the last cluster.
        let last = *chain.last().ok_or("empty directory chain")?;
        inner.fat_cache[last as usize] = new_cluster;
        inner.fat_cache[new_cluster as usize] = FAT_EOC;

        // Clear the new cluster and write the entry at slot 0.
        buf.fill(0);
        self.pack_entry(entry, &mut buf[..32]);
        if !self.write_cluster(inner, new_cluster, &buf) {
            return Err("ata write failed");
        }
        // Persist FAT changes.
        if !self.flush_fat(inner) {
            return Err("fat flush failed");
        }
        Ok(())
    }

    /// Remove a directory entry by exact 8.3 name (marks first byte 0xE5).
    fn remove_dir_entry(
        &self,
        inner: &mut Fat32Inner,
        dir_cluster: u32,
        short_name: &[u8; 11],
    ) -> Result<(), &'static str> {
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let chain = self.cluster_chain(inner, dir_cluster);
        let mut buf = vec![0u8; bytes_per_cluster];
        for c in chain {
            if !self.read_cluster(inner, c, &mut buf) {
                return Err("ata read failed");
            }
            for i in 0..bytes_per_cluster / 32 {
                let off = i * 32;
                if buf[off] == 0x00 {
                    // Reached end of directory — not found.
                    return Err("file not found");
                }
                if buf[off] == 0xE5 {
                    continue;
                }
                // Skip LFN entries.
                if buf[off + 11] == ATTR_LFN {
                    continue;
                }
                if &buf[off..off + 11] == short_name {
                    buf[off] = 0xE5;
                    if !self.write_cluster(inner, c, &buf) {
                        return Err("ata write failed");
                    }
                    return Ok(());
                }
            }
        }
        Err("file not found")
    }

    /// Pack a RawDirEntry into 32 raw bytes (little-endian).
    fn pack_entry(&self, e: &RawDirEntry, out: &mut [u8]) {
        out[..11].copy_from_slice(&e.name);
        out[11] = e.attr;
        out[12] = e.nt_res;
        out[13] = e.crt_time_tenth;
        out[14..16].copy_from_slice(&e.crt_time.to_le_bytes());
        out[16..18].copy_from_slice(&e.crt_date.to_le_bytes());
        out[18..20].copy_from_slice(&e.lst_acc_date.to_le_bytes());
        out[20..22].copy_from_slice(&e.fst_clus_hi.to_le_bytes());
        out[22..24].copy_from_slice(&e.wrt_time.to_le_bytes());
        out[24..26].copy_from_slice(&e.wrt_date.to_le_bytes());
        out[26..28].copy_from_slice(&e.fst_clus_lo.to_le_bytes());
        out[28..32].copy_from_slice(&e.file_size.to_le_bytes());
    }

    /// Convert a name to an 8.3 short name (uppercase, split on last dot).
    fn to_short_name(name: &str) -> [u8; 11] {
        let mut short = [b' '; 11];
        let (base, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], Some(&name[i + 1..])),
            _ => (name, None),
        };
        let base_upper = base.to_uppercase();
        let base_bytes = base_upper.as_bytes();
        let n = base_bytes.len().min(8);
        short[..n].copy_from_slice(&base_bytes[..n]);
        if let Some(e) = ext {
            let ext_upper = e.to_uppercase();
            let ext_bytes = ext_upper.as_bytes();
            let m = ext_bytes.len().min(3);
            short[8..8 + m].copy_from_slice(&ext_bytes[..m]);
        }
        short
    }

    /// Render a short name back to a String (trim spaces).
    fn from_short_name(short: &[u8; 11]) -> String {
        let mut s = String::new();
        let mut base_end = 0;
        for i in 0..8 {
            if short[i] == b' ' {
                break;
            }
            base_end = i + 1;
        }
        s.push_str(core::str::from_utf8(&short[..base_end]).unwrap_or(""));
        let mut ext_end = 0;
        for i in 8..11 {
            if short[i] == b' ' {
                break;
            }
            ext_end = i - 7;
        }
        if ext_end > 0 {
            s.push('.');
            s.push_str(core::str::from_utf8(&short[8..8 + ext_end]).unwrap_or(""));
        }
        s
    }

    /// Get or create an inode id for a (cluster, size, is_dir) tuple.
    fn intern_inode(&self, inner: &mut Fat32Inner, first_cluster: u32, size: u32, is_dir: bool) -> u64 {
        for (&id, ino) in &inner.inodes {
            if ino.first_cluster == first_cluster && ino.is_dir == is_dir {
                return id;
            }
        }
        let id = inner.next_inode;
        inner.next_inode += 1;
        inner.inodes.insert(
            id,
            FatInode {
                first_cluster,
                size,
                is_dir,
            },
        );
        id
    }
}

impl FileOps for Fat32 {
    fn read(&self, inode_id: u64, offset: u64, buf: &mut [u8]) -> Result<usize, &'static str> {
        let inner = self.inner.lock();
        let inode = *inner.inodes.get(&inode_id).ok_or("inode not found")?;
        if inode.is_dir || buf.is_empty() {
            return Ok(0);
        }
        if offset >= inode.size as u64 {
            return Ok(0);
        }
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let chain = self.cluster_chain(&inner, inode.first_cluster);
        let mut read_total = 0usize;
        let mut pos = offset;
        let end = (inode.size as u64).min(offset + buf.len() as u64);
        let mut scratch = vec![0u8; bytes_per_cluster];
        while pos < end {
            let rel = (pos - offset) as usize;
            let cluster_idx = (pos / bytes_per_cluster as u64) as usize;
            if cluster_idx >= chain.len() {
                break;
            }
            let in_cluster = (pos % bytes_per_cluster as u64) as usize;
            if !self.read_cluster(&inner, chain[cluster_idx], &mut scratch) {
                return Err("ata read failed");
            }
            let take = ((end - pos) as usize).min(bytes_per_cluster - in_cluster);
            buf[rel..rel + take].copy_from_slice(&scratch[in_cluster..in_cluster + take]);
            pos += take as u64;
            read_total += take;
        }
        Ok(read_total)
    }

    fn write(&self, inode_id: u64, offset: u64, buf: &[u8]) -> Result<usize, &'static str> {
        let mut inner = self.inner.lock();
        let mut inode = *inner.inodes.get(&inode_id).ok_or("inode not found")?;
        if inode.is_dir {
            return Err("cannot write to directory");
        }
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let end = offset + buf.len() as u64;

        // Ensure the cluster chain covers the write range.
        let mut chain = self.cluster_chain(&inner, inode.first_cluster);
        let needed_clusters = if end == 0 {
            0
        } else {
            ((end + bytes_per_cluster as u64 - 1) / bytes_per_cluster as u64) as usize
        };
        while chain.len() < needed_clusters {
            let new_cluster = self.alloc_cluster(&mut inner).ok_or("out of clusters")?;
            if chain.is_empty() {
                inode.first_cluster = new_cluster;
                inner.inodes.get_mut(&inode_id).unwrap().first_cluster = new_cluster;
            } else {
                let last = *chain.last().unwrap();
                inner.fat_cache[last as usize] = new_cluster;
            }
            chain.push(new_cluster);
        }

        // If the file was empty (first_cluster=0), we just allocated a chain.
        if inode.first_cluster == 0 && !chain.is_empty() {
            inner.inodes.get_mut(&inode_id).unwrap().first_cluster = chain[0];
        }

        // Write data cluster by cluster.
        let mut pos = offset;
        let mut written = 0usize;
        let mut scratch = vec![0u8; bytes_per_cluster];
        while pos < end {
            let cluster_idx = ((pos - offset) / bytes_per_cluster as u64) as usize;
            if cluster_idx >= chain.len() {
                break;
            }
            let in_cluster = (pos % bytes_per_cluster as u64) as usize;
            // Read-modify-write: load existing cluster data first.
            if !self.read_cluster(&inner, chain[cluster_idx], &mut scratch) {
                return Err("ata read failed");
            }
            let take = ((end - pos) as usize).min(bytes_per_cluster - in_cluster);
            scratch[in_cluster..in_cluster + take].copy_from_slice(&buf[written..written + take]);
            if !self.write_cluster(&inner, chain[cluster_idx], &scratch) {
                return Err("ata write failed");
            }
            pos += take as u64;
            written += take;
        }

        // Update size if we grew the file.
        if end > inode.size as u64 {
            inner.inodes.get_mut(&inode_id).unwrap().size = end as u32;
        }
        // Persist FAT changes.
        if !self.flush_fat(&inner) {
            return Err("fat flush failed");
        }
        // Update the on-disk directory entry size.
        self.update_dir_entry_size(&mut inner, inode_id)?;
        Ok(written)
    }

    fn size(&self, inode_id: u64) -> u64 {
        self.inner
            .lock()
            .inodes
            .get(&inode_id)
            .map(|i| i.size as u64)
            .unwrap_or(0)
    }

    fn file_type(&self, inode_id: u64) -> FileType {
        self.inner
            .lock()
            .inodes
            .get(&inode_id)
            .map(|i| {
                if i.is_dir {
                    FileType::Directory
                } else {
                    FileType::Regular
                }
            })
            .unwrap_or(FileType::Regular)
    }

    fn readdir(&self, inode_id: u64) -> Result<Vec<DirEntry>, &'static str> {
        let mut inner = self.inner.lock();
        let inode = *inner.inodes.get(&inode_id).ok_or("inode not found")?;
        if !inode.is_dir {
            return Err("not a directory");
        }
        let raw = self.read_dir_raw(&inner, inode.first_cluster);
        let mut out = Vec::new();
        for e in &raw {
            if e.is_free() || e.is_lfn() {
                continue;
            }
            // Skip "." and ".." entries.
            if e.name[0] == b'.' {
                continue;
            }
            let name = Self::from_short_name(&e.name);
            let first = e.first_cluster();
            let id = self.intern_inode(&mut inner, first, e.file_size, e.is_dir());
            out.push(DirEntry {
                name,
                inode_id: id,
                file_type: if e.is_dir() {
                    FileType::Directory
                } else {
                    FileType::Regular
                },
            });
        }
        Ok(out)
    }

    fn lookup(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        let mut inner = self.inner.lock();
        let dir = *inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
        if !dir.is_dir {
            return Err("not a directory");
        }
        let short = Self::to_short_name(name);
        let raw = self.read_dir_raw(&inner, dir.first_cluster);
        for e in &raw {
            if e.is_free() || e.is_lfn() {
                continue;
            }
            if &e.name == &short {
                let first = e.first_cluster();
                let id = self.intern_inode(&mut inner, first, e.file_size, e.is_dir());
                return Ok(id);
            }
        }
        Err("file not found")
    }

    fn create(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        let mut inner = self.inner.lock();
        let dir = *inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
        if !dir.is_dir {
            return Err("not a directory");
        }
        // Reject duplicate names.
        let short = Self::to_short_name(name);
        let existing = self.read_dir_raw(&inner, dir.first_cluster);
        for e in &existing {
            if !e.is_free() && !e.is_lfn() && &e.name == &short {
                return Err("file already exists");
            }
        }
        let entry = RawDirEntry {
            name: short,
            attr: ATTR_ARCHIVE,
            nt_res: 0,
            crt_time_tenth: 0,
            crt_time: 0,
            crt_date: 0,
            lst_acc_date: 0,
            fst_clus_hi: 0,
            wrt_time: 0,
            wrt_date: 0,
            fst_clus_lo: 0,
            file_size: 0,
        };
        self.write_dir_entry(&mut inner, dir.first_cluster, &entry, true)?;
        if !self.flush_fat(&inner) {
            return Err("fat flush failed");
        }
        let id = self.intern_inode(&mut inner, 0, 0, false);
        Ok(id)
    }

    fn mkdir(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        let mut inner = self.inner.lock();
        let dir = *inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
        if !dir.is_dir {
            return Err("not a directory");
        }
        let short = Self::to_short_name(name);
        let existing = self.read_dir_raw(&inner, dir.first_cluster);
        for e in &existing {
            if !e.is_free() && !e.is_lfn() && &e.name == &short {
                return Err("dir already exists");
            }
        }
        let new_cluster = self.alloc_cluster(&mut inner).ok_or("out of clusters")?;

        // Create "." and ".." entries in the new directory.
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let mut buf = vec![0u8; bytes_per_cluster];
        let dot = RawDirEntry {
            name: *b".          ",
            attr: ATTR_DIR,
            nt_res: 0,
            crt_time_tenth: 0,
            crt_time: 0,
            crt_date: 0,
            lst_acc_date: 0,
            fst_clus_hi: (new_cluster >> 16) as u16,
            wrt_time: 0,
            wrt_date: 0,
            fst_clus_lo: (new_cluster & 0xFFFF) as u16,
            file_size: 0,
        };
        self.pack_entry(&dot, &mut buf[..32]);
        let dotdot = RawDirEntry {
            name: *b"..         ",
            attr: ATTR_DIR,
            nt_res: 0,
            crt_time_tenth: 0,
            crt_time: 0,
            crt_date: 0,
            lst_acc_date: 0,
            fst_clus_hi: (dir.first_cluster >> 16) as u16,
            wrt_time: 0,
            wrt_date: 0,
            fst_clus_lo: (dir.first_cluster & 0xFFFF) as u16,
            file_size: 0,
        };
        self.pack_entry(&dotdot, &mut buf[32..64]);
        if !self.write_cluster(&inner, new_cluster, &buf) {
            return Err("ata write failed");
        }

        // Add the directory entry to the parent.
        let entry = RawDirEntry {
            name: short,
            attr: ATTR_DIR,
            nt_res: 0,
            crt_time_tenth: 0,
            crt_time: 0,
            crt_date: 0,
            lst_acc_date: 0,
            fst_clus_hi: (new_cluster >> 16) as u16,
            wrt_time: 0,
            wrt_date: 0,
            fst_clus_lo: (new_cluster & 0xFFFF) as u16,
            file_size: 0,
        };
        self.write_dir_entry(&mut inner, dir.first_cluster, &entry, true)?;
        if !self.flush_fat(&inner) {
            return Err("fat flush failed");
        }
        let id = self.intern_inode(&mut inner, new_cluster, 0, true);
        Ok(id)
    }

    fn truncate(&self, inode_id: u64, size: u64) -> Result<(), &'static str> {
        let mut inner = self.inner.lock();
        let inode = *inner.inodes.get(&inode_id).ok_or("inode not found")?;
        if inode.is_dir {
            return Err("cannot truncate directory");
        }
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let new_size = size as u32;
        let needed = if new_size == 0 {
            0
        } else {
            ((new_size as u64 + bytes_per_cluster as u64 - 1) / bytes_per_cluster as u64) as usize
        };
        let chain = self.cluster_chain(&inner, inode.first_cluster);
        if needed < chain.len() && !chain.is_empty() {
            // Free trailing clusters.
            let keep = needed.max(if new_size == 0 { 0 } else { 1 });
            for &c in &chain[keep..] {
                inner.fat_cache[c as usize] = FAT_FREE;
            }
            // If new_size is 0, free everything and reset first_cluster.
            if new_size == 0 {
                inner.fat_cache[chain[0] as usize] = FAT_FREE;
                inner.inodes.get_mut(&inode_id).unwrap().first_cluster = 0;
            } else {
                // Mark the last kept cluster as EOC.
                inner.fat_cache[chain[keep - 1] as usize] = FAT_EOC;
            }
        }
        inner.inodes.get_mut(&inode_id).unwrap().size = new_size;
        if !self.flush_fat(&inner) {
            return Err("fat flush failed");
        }
        self.update_dir_entry_size(&mut inner, inode_id)?;
        Ok(())
    }

    fn unlink(&self, dir_inode_id: u64, name: &str) -> Result<(), &'static str> {
        let mut inner = self.inner.lock();
        let dir = *inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
        if !dir.is_dir {
            return Err("not a directory");
        }
        let short = Self::to_short_name(name);
        // Find the target entry to free its clusters.
        let raw = self.read_dir_raw(&inner, dir.first_cluster);
        let mut target: Option<RawDirEntry> = None;
        for e in &raw {
            if !e.is_free() && !e.is_lfn() && &e.name == &short {
                target = Some(RawDirEntry {
                    name: e.name,
                    attr: e.attr,
                    nt_res: e.nt_res,
                    crt_time_tenth: e.crt_time_tenth,
                    crt_time: e.crt_time,
                    crt_date: e.crt_date,
                    lst_acc_date: e.lst_acc_date,
                    fst_clus_hi: e.fst_clus_hi,
                    wrt_time: e.wrt_time,
                    wrt_date: e.wrt_date,
                    fst_clus_lo: e.fst_clus_lo,
                    file_size: e.file_size,
                });
                break;
            }
        }
        let t = target.ok_or("file not found")?;
        // Free the cluster chain.
        if t.first_cluster() >= 2 {
            self.free_chain(&mut inner, t.first_cluster());
        }
        // Remove the directory entry.
        self.remove_dir_entry(&mut inner, dir.first_cluster, &short)?;
        if !self.flush_fat(&inner) {
            return Err("fat flush failed");
        }
        // Drop the inode from our table.
        let mut victim = None;
        for (&id, ino) in &inner.inodes {
            if ino.first_cluster == t.first_cluster() && ino.is_dir == t.is_dir() {
                victim = Some(id);
                break;
            }
        }
        if let Some(id) = victim {
            inner.inodes.remove(&id);
        }
        Ok(())
    }
}

impl Fat32 {
    /// Update the file size field of a file's directory entry on disk.
    fn update_dir_entry_size(&self, inner: &mut Fat32Inner, inode_id: u64) -> Result<(), &'static str> {
        let inode = *inner.inodes.get(&inode_id).ok_or("inode not found")?;
        if inode.is_dir {
            return Ok(());
        }
        let target_cluster = inode.first_cluster;
        self.update_size_recursive(inner, inner.root_cluster, target_cluster, inode.size)?;
        Ok(())
    }

    fn update_size_recursive(
        &self,
        inner: &mut Fat32Inner,
        dir_cluster: u32,
        target_cluster: u32,
        new_size: u32,
    ) -> Result<bool, &'static str> {
        let bytes_per_cluster = inner.sectors_per_cluster as usize * inner.bytes_per_sector as usize;
        let mut buf = vec![0u8; bytes_per_cluster];
        let chain = self.cluster_chain(inner, dir_cluster);
        let mut subdirs = Vec::new();
        for c in chain {
            if !self.read_cluster(inner, c, &mut buf) {
                return Err("ata read failed");
            }
            for i in 0..bytes_per_cluster / 32 {
                let off = i * 32;
                if buf[off] == 0x00 || buf[off] == 0xE5 || buf[off + 11] == ATTR_LFN {
                    continue;
                }
                let name = &buf[off..off + 11];
                if name[0] == b'.' {
                    continue;
                }
                let fst_clus_hi = u16::from_le_bytes([buf[off + 20], buf[off + 21]]);
                let fst_clus_lo = u16::from_le_bytes([buf[off + 26], buf[off + 27]]);
                let cluster = ((fst_clus_hi as u32) << 16) | fst_clus_lo as u32;
                if cluster == target_cluster {
                    buf[off + 28..off + 32].copy_from_slice(&new_size.to_le_bytes());
                    if !self.write_cluster(inner, c, &buf) {
                        return Err("ata write failed");
                    }
                    return Ok(true);
                }
                if buf[off + 11] & ATTR_DIR != 0 && cluster >= 2 {
                    subdirs.push(cluster);
                }
            }
        }
        for d in subdirs {
            if self.update_size_recursive(inner, d, target_cluster, new_size)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Scan the MBR for a FAT32 partition and return its start LBA + size.
///
/// The bootloader 0.11 also creates a small FAT32 partition of its own;
/// our data partition is always appended last, so we scan from the end
/// and return the final FAT32 partition.
pub fn find_fat32_partition() -> Option<(u64, u64)> {
    let mut mbr = [0u8; 512];
    if !crate::ata::read_sector(0, &mut mbr) {
        return None;
    }
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return None;
    }
    for i in (0..4).rev() {
        let off = 446 + i * 16;
        let ptype = mbr[off + 4];
        if ptype == PART_TYPE_FAT32 || ptype == PART_TYPE_FAT32_CHS {
            let start =
                u32::from_le_bytes([mbr[off + 8], mbr[off + 9], mbr[off + 10], mbr[off + 11]]) as u64;
            let size =
                u32::from_le_bytes([mbr[off + 12], mbr[off + 13], mbr[off + 14], mbr[off + 15]]) as u64;
            return Some((start, size));
        }
    }
    None
}

/// Initialise the FAT32 filesystem: locate the partition, parse the BPB,
/// load the FAT, and mount into the VFS at `/disk`.
pub fn init() -> bool {
    if !crate::ata::disk_present() {
        crate::serial::_print(format_args!("[fat32] skipped — no disk present\n"));
        return false;
    }
    let (part_start, part_size) = match find_fat32_partition() {
        Some(p) => p,
        None => {
            crate::serial::_print(format_args!("[fat32] no FAT32 partition found in MBR\n"));
            return false;
        }
    };

    // Read the BPB.
    let mut bpb = [0u8; 512];
    if !crate::ata::read_sector(part_start, &mut bpb) {
        crate::serial::_print(format_args!("[fat32] BPB read failed\n"));
        return false;
    }
    if !(bpb[510] == 0x55 && bpb[511] == 0xAA) {
        crate::serial::_print(format_args!("[fat32] bad boot signature\n"));
        return false;
    }
    let bytes_per_sector = u16::from_le_bytes([bpb[11], bpb[12]]);
    let sectors_per_cluster = bpb[13];
    let rsvd_sec_cnt = u16::from_le_bytes([bpb[14], bpb[15]]);
    let num_fats = bpb[16];
    let fat_sz32 = u32::from_le_bytes([bpb[36], bpb[37], bpb[38], bpb[39]]);
    let root_cluster = u32::from_le_bytes([bpb[44], bpb[45], bpb[46], bpb[47]]);

    if bytes_per_sector != 512 || sectors_per_cluster == 0 || fat_sz32 == 0 {
        crate::serial::_print(format_args!(
            "[fat32] unsupported BPB: bps={} spc={} fatsz={}\n",
            bytes_per_sector, sectors_per_cluster, fat_sz32
        ));
        return false;
    }

    let data_start_lba = part_start + rsvd_sec_cnt as u64 + num_fats as u64 * fat_sz32 as u64;

    crate::serial::_print(format_args!(
        "[fat32] partition @ LBA {} ({} sectors): bps={} spc={} fatsz={} root={}\n",
        part_start, part_size, bytes_per_sector, sectors_per_cluster, fat_sz32, root_cluster
    ));

    let fs = Fat32 {
        inner: Mutex::new(Fat32Inner {
            part_start_lba: part_start,
            bytes_per_sector,
            sectors_per_cluster,
            rsvd_sec_cnt,
            num_fats,
            fat_sz32,
            root_cluster,
            data_start_lba,
            fat_cache: Vec::new(),
            inodes: BTreeMap::new(),
            next_inode: 1,
        }),
    };

    // Load the FAT.
    {
        let mut inner = fs.inner.lock();
        if !fs.load_fat(&mut inner) {
            crate::serial::_print(format_args!("[fat32] FAT load failed\n"));
            return false;
        }
        // Intern the root directory inode (id 1).
        fs.intern_inode(&mut inner, root_cluster, 0, true);
    }

    // Mount at /disk (root inode is id 1).
    let total_clusters = fs.inner.lock().fat_cache.len().saturating_sub(2);
    let boxed: Box<dyn FileOps> = Box::new(fs);
    VFS.lock().mount("/disk", 1, boxed);

    crate::serial::_print(format_args!(
        "[fat32] mounted FAT32 at /disk ({} clusters total)\n",
        total_clusters
    ));
    true
}
