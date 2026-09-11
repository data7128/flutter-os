//! Virtual File System (VFS) layer.
//!
//! Provides a unified interface for all filesystems (tmpfs, future FAT32,
//! devfs, etc.). All FileOps methods take `&self` — filesystems use
//! interior mutability (Mutex) internally.

#![allow(dead_code)]

pub mod devfs;
pub mod fat32;
pub mod initramfs;
pub mod tmpfs;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

/// File type for an inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    CharDevice,
    BlockDevice,
}

/// A directory entry returned by readdir.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub inode_id: u64,
    pub file_type: FileType,
}

/// Filesystem-specific file operations. All methods take &self;
/// implementations use interior mutability.
pub trait FileOps: Send + Sync {
    fn read(&self, inode_id: u64, offset: u64, buf: &mut [u8]) -> Result<usize, &'static str>;
    fn write(&self, inode_id: u64, offset: u64, buf: &[u8]) -> Result<usize, &'static str>;
    fn size(&self, inode_id: u64) -> u64;
    fn file_type(&self, inode_id: u64) -> FileType;
    fn readdir(&self, inode_id: u64) -> Result<Vec<DirEntry>, &'static str>;
    fn lookup(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str>;
    fn create(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str>;
    fn mkdir(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str>;
    fn truncate(&self, inode_id: u64, size: u64) -> Result<(), &'static str>;
    fn unlink(&self, dir_inode_id: u64, name: &str) -> Result<(), &'static str>;
}

/// A mounted filesystem.
pub struct Mount {
    pub mount_point: String,
    pub root_inode: u64,
    pub ops: Box<dyn FileOps>,
}

/// An open file in the VFS.
pub struct VfsFile {
    pub inode_id: u64,
    pub mount_idx: usize,
    pub offset: u64,
    pub file_type: FileType,
}

/// Global VFS state.
pub struct Vfs {
    pub mounts: Vec<Mount>,
}

impl Vfs {
    pub const fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    pub fn mount(&mut self, mount_point: &str, root_inode: u64, ops: Box<dyn FileOps>) {
        self.mounts.push(Mount {
            mount_point: String::from(mount_point),
            root_inode,
            ops,
        });
        self.mounts.sort_by(|a, b| b.mount_point.len().cmp(&a.mount_point.len()));
    }

    pub fn find_mount<'a>(&'a self, path: &'a str) -> Option<(usize, &'a str)> {
        for (i, mount) in self.mounts.iter().enumerate() {
            let matched = if mount.mount_point == "/" {
                path.starts_with('/')
            } else {
                path == mount.mount_point
                    || path.starts_with(&format!("{}/", mount.mount_point))
            };
            if matched {
                let relative = if path == mount.mount_point {
                    ""
                } else if mount.mount_point == "/" {
                    &path[1..]
                } else {
                    &path[mount.mount_point.len()..]
                };
                return Some((i, relative));
            }
        }
        None
    }

    pub fn resolve(&self, path: &str) -> Result<(usize, u64), &'static str> {
        let (mount_idx, relative) = self.find_mount(path).ok_or("no mount for path")?;
        let mount = &self.mounts[mount_idx];
        if relative.is_empty() || relative == "/" {
            return Ok((mount_idx, mount.root_inode));
        }
        let mut current_inode = mount.root_inode;
        let components: Vec<&str> = relative.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
        for component in components {
            if mount.ops.file_type(current_inode) != FileType::Directory {
                return Err("not a directory");
            }
            current_inode = mount.ops.lookup(current_inode, component)?;
        }
        Ok((mount_idx, current_inode))
    }

    /// Split `/a/b/c` into (`/a/b`, `c`); `/bin` into (`/`, `bin`); `bin` into (`/`, `bin`).
    fn split_parent_name<'a>(&self, path: &'a str) -> (&'a str, &'a str) {
        match path.rfind('/') {
            Some(0) => ("/", &path[1..]),
            Some(i) => (&path[..i], &path[i + 1..]),
            None => ("/", path),
        }
    }

    pub fn open(&self, path: &str, create: bool) -> Result<VfsFile, &'static str> {
        let (mount_idx, inode_id) = match self.resolve(path) {
            Ok(v) => v,
            Err(_) if create => {
                let (mount_idx, _) = self.find_mount(path).ok_or("no mount for path")?;
                let (parent, name) = self.split_parent_name(path);
                let (_, parent_inode) = self.resolve(parent)?;
                let new_inode = self.mounts[mount_idx].ops.create(parent_inode, name)?;
                (mount_idx, new_inode)
            }
            Err(e) => return Err(e),
        };
        let file_type = self.mounts[mount_idx].ops.file_type(inode_id);
        Ok(VfsFile { inode_id, mount_idx, offset: 0, file_type })
    }

    pub fn read(&self, file: &mut VfsFile, buf: &mut [u8]) -> Result<usize, &'static str> {
        let n = self.mounts[file.mount_idx].ops.read(file.inode_id, file.offset, buf)?;
        file.offset += n as u64;
        Ok(n)
    }

    pub fn write(&self, file: &mut VfsFile, buf: &[u8]) -> Result<usize, &'static str> {        let n = self.mounts[file.mount_idx].ops.write(file.inode_id, file.offset, buf)?;
        file.offset += n as u64;
        Ok(n)
    }

    pub fn size(&self, file: &VfsFile) -> u64 {
        self.mounts[file.mount_idx].ops.size(file.inode_id)
    }

    pub fn read_all(&self, path: &str) -> Result<Vec<u8>, &'static str> {
        let mut file = self.open(path, false)?;
        let size = self.size(&file);
        let mut buf = vec![0u8; size as usize];
        let mut total = 0usize;
        while total < buf.len() {
            let n = self.read(&mut file, &mut buf[total..])?;
            if n == 0 { break; }
            total += n;
        }
        buf.truncate(total);
        Ok(buf)
    }

    pub fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, &'static str> {
        let (mount_idx, inode_id) = self.resolve(path)?;
        self.mounts[mount_idx].ops.readdir(inode_id)
    }
}

pub static VFS: Mutex<Vfs> = Mutex::new(Vfs::new());

pub fn init() {
    crate::serial::_print(format_args!("[vfs] virtual filesystem layer initialised\n"));
}
