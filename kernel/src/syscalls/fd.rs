//! File descriptor table management.
//!
//! Maps small integer descriptors to open file/stream entries.
//! Each entry tracks the type (file, stream, framebuffer) and an
//! internal offset for seekable objects.
//!
//! ← FUTURE: when per-process address spaces exist, each `Process`
//! struct will own its own `FdTable`. For now, a single global table
//! suffices.

/// Maximum number of open FDs in the global table.
pub const MAX_FDS: usize = 128;

/// Reserved standard FDs.
pub const STDIN_FD: usize = 0;
pub const STDOUT_FD: usize = 1;
pub const STDERR_FD: usize = 2;

/// Type of a file descriptor entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdKind {
    /// Reserved standard stream (stdin/stdout/stderr).
    Stream,
    /// FAT32 file on ATA disk (not yet implemented).
    File { sector: u64, offset: u64, size: u64 },
    /// Framebuffer device (/dev/fb0 equivalent).
    Framebuffer,
    /// Free / unused slot.
    Free,
}

/// A single file descriptor entry.
#[derive(Debug, Clone, Copy)]
pub struct FdEntry {
    pub kind: FdKind,
}

impl Default for FdEntry {
    fn default() -> Self {
        Self {
            kind: FdKind::Free,
        }
    }
}

/// Global file descriptor table.
///
/// Indices 0, 1, 2 are pre-allocated for stdin/stdout/stderr.
/// File-backed FDs start at index 3.
///
/// ← FUTURE: when FAT32 is implemented, `open()` will allocate an
/// `FdKind::File` entry here.
pub struct FdTable {
    entries: [FdEntry; MAX_FDS],
}

impl FdTable {
    /// Create an empty FD table with std streams pre-allocated.
    pub const fn new() -> Self {
        let mut entries = [FdEntry {
            kind: FdKind::Free,
        }; MAX_FDS];
        entries[STDIN_FD] = FdEntry {
            kind: FdKind::Stream,
        };
        entries[STDOUT_FD] = FdEntry {
            kind: FdKind::Stream,
        };
        entries[STDERR_FD] = FdEntry {
            kind: FdKind::Stream,
        };
        Self { entries }
    }

    /// Allocate a new FD entry. Returns the index.
    /// Returns `None` if the table is full.
    pub fn alloc(&mut self, kind: FdKind) -> Option<usize> {
        for i in 3..MAX_FDS {
            if self.entries[i].kind == FdKind::Free {
                self.entries[i] = FdEntry { kind };
                return Some(i);
            }
        }
        None
    }

    /// Get a reference to an FD entry.
    pub fn get(&self, fd: usize) -> Option<&FdEntry> {
        if fd < MAX_FDS && self.entries[fd].kind != FdKind::Free {
            Some(&self.entries[fd])
        } else {
            None
        }
    }

    /// Close (free) an FD entry.
    pub fn close(&mut self, fd: usize) -> bool {
        if fd >= 3 && fd < MAX_FDS && self.entries[fd].kind != FdKind::Free {
            self.entries[fd] = FdEntry {
                kind: FdKind::Free,
            };
            true
        } else {
            false
        }
    }

    /// Count open FDs (excluding std streams).
    pub fn open_count(&self) -> usize {
        self.entries[3..]
            .iter()
            .filter(|e| e.kind != FdKind::Free)
            .count()
    }
}
