//! tmpfs — in-memory temporary filesystem with interior mutability.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::{DirEntry, FileOps, FileType, VFS};

struct TmpfsInode {
    file_type: FileType,
    data: Vec<u8>,
    children: BTreeMap<String, u64>,
}

struct TmpfsInner {
    inodes: BTreeMap<u64, TmpfsInode>,
    next_inode: u64,
    root_inode: u64,
}

pub struct Tmpfs {
    inner: Mutex<TmpfsInner>,
}

impl Tmpfs {
    pub fn new() -> Self {
        let mut inodes = BTreeMap::new();
        inodes.insert(1, TmpfsInode {
            file_type: FileType::Directory,
            data: Vec::new(),
            children: BTreeMap::new(),
        });
        Self {
            inner: Mutex::new(TmpfsInner {
                inodes,
                next_inode: 2,
                root_inode: 1,
            }),
        }
    }

    pub fn root_inode(&self) -> u64 { 1 }
}

impl FileOps for Tmpfs {
    fn read(&self, inode_id: u64, offset: u64, buf: &mut [u8]) -> Result<usize, &'static str> {
        let inner = self.inner.lock();
        let inode = inner.inodes.get(&inode_id).ok_or("inode not found")?;
        if inode.file_type != FileType::Regular { return Err("not a regular file"); }
        let start = offset as usize;
        if start >= inode.data.len() { return Ok(0); }
        let end = (start + buf.len()).min(inode.data.len());
        buf[..end - start].copy_from_slice(&inode.data[start..end]);
        Ok(end - start)
    }

    fn write(&self, inode_id: u64, offset: u64, buf: &[u8]) -> Result<usize, &'static str> {
        let mut inner = self.inner.lock();
        let inode = inner.inodes.get_mut(&inode_id).ok_or("inode not found")?;
        if inode.file_type != FileType::Regular { return Err("not a regular file"); }
        let start = offset as usize;
        let end = start + buf.len();
        if end > inode.data.len() { inode.data.resize(end, 0); }
        inode.data[start..end].copy_from_slice(buf);
        Ok(buf.len())
    }

    fn size(&self, inode_id: u64) -> u64 {
        self.inner.lock().inodes.get(&inode_id).map(|i| i.data.len() as u64).unwrap_or(0)
    }

    fn file_type(&self, inode_id: u64) -> FileType {
        self.inner.lock().inodes.get(&inode_id).map(|i| i.file_type).unwrap_or(FileType::Regular)
    }

    fn readdir(&self, inode_id: u64) -> Result<Vec<DirEntry>, &'static str> {
        let inner = self.inner.lock();
        // Collect (name, id) pairs in a scoped borrow.
        let children: Vec<(String, u64)> = {
            let inode = inner.inodes.get(&inode_id).ok_or("inode not found")?;
            if inode.file_type != FileType::Directory { return Err("not a directory"); }
            inode.children.iter()
                .map(|(name, &id)| (name.clone(), id))
                .collect()
        };
        // Build DirEntry entries, looking up file_type separately.
        let mut entries = Vec::new();
        for (name, id) in children {
            let ft = inner.inodes.get(&id).map(|c| c.file_type).unwrap_or(FileType::Regular);
            entries.push(DirEntry { name, inode_id: id, file_type: ft });
        }
        Ok(entries)
    }

    fn lookup(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        let inner = self.inner.lock();
        let dir = inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
        if dir.file_type != FileType::Directory { return Err("not a directory"); }
        dir.children.get(name).copied().ok_or("file not found")
    }

    fn create(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        let mut inner = self.inner.lock();
        // Validate directory exists and is a directory.
        {
            let dir = inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
            if dir.file_type != FileType::Directory { return Err("not a directory"); }
            if dir.children.contains_key(name) { return Err("file already exists"); }
        }
        // Allocate new inode.
        let new_id = inner.next_inode;
        inner.next_inode += 1;
        inner.inodes.insert(new_id, TmpfsInode {
            file_type: FileType::Regular, data: Vec::new(), children: BTreeMap::new(),
        });
        // Add to parent directory.
        let dir = inner.inodes.get_mut(&dir_inode_id).unwrap();
        dir.children.insert(String::from(name), new_id);
        Ok(new_id)
    }

    fn mkdir(&self, dir_inode_id: u64, name: &str) -> Result<u64, &'static str> {
        let mut inner = self.inner.lock();
        {
            let dir = inner.inodes.get(&dir_inode_id).ok_or("inode not found")?;
            if dir.file_type != FileType::Directory { return Err("not a directory"); }
            if dir.children.contains_key(name) { return Err("dir already exists"); }
        }
        let new_id = inner.next_inode;
        inner.next_inode += 1;
        inner.inodes.insert(new_id, TmpfsInode {
            file_type: FileType::Directory, data: Vec::new(), children: BTreeMap::new(),
        });
        let dir = inner.inodes.get_mut(&dir_inode_id).unwrap();
        dir.children.insert(String::from(name), new_id);
        Ok(new_id)
    }

    fn truncate(&self, inode_id: u64, size: u64) -> Result<(), &'static str> {
        let mut inner = self.inner.lock();
        let inode = inner.inodes.get_mut(&inode_id).ok_or("inode not found")?;
        if inode.file_type != FileType::Regular { return Err("not a regular file"); }
        inode.data.resize(size as usize, 0);
        Ok(())
    }

    fn unlink(&self, dir_inode_id: u64, name: &str) -> Result<(), &'static str> {
        let mut inner = self.inner.lock();
        let dir = inner.inodes.get_mut(&dir_inode_id).ok_or("inode not found")?;
        if dir.file_type != FileType::Directory { return Err("not a directory"); }
        let child_id = dir.children.remove(name).ok_or("file not found")?;
        inner.inodes.remove(&child_id);
        Ok(())
    }
}

/// Initialise tmpfs and mount it as the root filesystem.
pub fn init() {
    let tmpfs = Tmpfs::new();
    let root = tmpfs.root_inode();
    let boxed: alloc::boxed::Box<dyn FileOps> = alloc::boxed::Box::new(tmpfs);
    VFS.lock().mount("/", root, boxed);
    crate::serial::_print(format_args!("[tmpfs] mounted as root filesystem\n"));
}

/// Create a file at `path` and write `data` into it (convenience helper).
pub fn create_file_with_data(path: &str, data: &[u8]) -> Result<(), &'static str> {
    let vfs = VFS.lock();
    let (mount_idx, _) = vfs.find_mount(path).ok_or("no mount")?;
    let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
    let name = path.rsplit('/').next().unwrap_or("");
    let (_, parent_inode) = vfs.resolve(parent)?;
    let inode_id = vfs.mounts[mount_idx].ops.create(parent_inode, name)?;
    vfs.mounts[mount_idx].ops.write(inode_id, 0, data)?;
    Ok(())
}

/// Create a directory at `path`.
pub fn mkdir(path: &str) -> Result<(), &'static str> {
    let vfs = VFS.lock();
    let (mount_idx, _) = vfs.find_mount(path).ok_or("no mount")?;
    let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
    let name = path.rsplit('/').next().unwrap_or("");
    let (_, parent_inode) = vfs.resolve(parent)?;
    vfs.mounts[mount_idx].ops.mkdir(parent_inode, name)?;
    Ok(())
}
