//! A memory-mapped source for the PVec reader (issue #108, ADR 0016).
//!
//! `pvec-container` is an API crate: it opens no file and contains no `unsafe` beyond the casts in
//! `blocks/aligned.rs`. Mapping a file cannot be written in safe Rust, so it is here, in a crate of its
//! own, with one `unsafe` block in `src/map.rs` (the only other module is this one, and it denies
//! `unsafe_code`). The map comes from `memmap2`.
//!
//! What a map gives the reader is the same as what any other source gives it: untrusted bytes. The
//! source checks every range against the length the file had when it was mapped before it touches the
//! map, returns a copy of the range (never a borrow of the map), and the reader validates the copy like
//! any other. The corpus of hostile files runs through it with the same verdicts as through memory.
//!
//! # When to use it, and when not to
//!
//! A map is fastest when the same file is read many times, and it shares pages between processes. It is
//! the wrong choice for a file that something else can write to or truncate while it is mapped: the
//! operating system answers a touch of a page that is no longer in the file with a signal (SIGBUS on
//! Linux and macOS), which a Rust program cannot recover from. This source compares the file's current
//! length with the mapped one before each read and returns [`SourceError::Short`] when it has shrunk, so
//! a file cut off in between reads is an error. It cannot close the window between that check and the
//! copy. So: map a file this process owns, or a read-only copy, and use `pvec_sources::FileSource` or
//! `ReadAtSource` over a `PositionalFile` for anything else, because those return an error for a file
//! that changes, in every case.
#![deny(unsafe_code)]

mod map;

use memmap2::Mmap;
use pvec_container::source::{BlockSource, Bytes, SourceError};
use std::fs::File;
use std::path::Path;

fn io(e: std::io::Error) -> SourceError {
    SourceError::Io(e.to_string())
}

/// A local file mapped into memory, read-only. The length is the file's when it was mapped.
#[derive(Debug)]
pub struct MmapSource {
    // `None` for an empty file, which cannot be mapped.
    map: Option<Mmap>,
    file: File,
    len: u64,
}

impl MmapSource {
    /// Maps the regular file at `path` read-only. A missing file, a directory or a file that cannot be
    /// mapped is [`SourceError::Io`].
    pub fn open(path: impl AsRef<Path>) -> Result<MmapSource, SourceError> {
        let file = File::open(path).map_err(io)?;
        let meta = file.metadata().map_err(io)?;
        if !meta.is_file() {
            return Err(SourceError::Io("not a regular file".into()));
        }
        let len = meta.len();
        let map = if len == 0 { None } else { Some(map::map_read_only(&file).map_err(io)?) };
        Ok(MmapSource { map, file, len })
    }
}

impl BlockSource for MmapSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        let end = match offset.checked_add(u64::from(len)) {
            Some(end) if end <= self.len => end,
            _ => return Err(SourceError::OutOfRange { offset, len: u64::from(len), source_len: self.len }),
        };
        let Some(map) = &self.map else {
            // only a zero-length range of an empty source gets here, and the check above passed it
            return Ok(Bytes::new(Vec::new()));
        };
        // a file that is shorter than the map is not read: the pages past its end are not backed
        let now = self.file.metadata().map_err(io)?.len();
        if now < self.len {
            return Err(SourceError::Short {
                wanted: u64::from(len),
                got: now.saturating_sub(offset).min(u64::from(len)),
            });
        }
        // in range by the check above, and `end` is at most the mapped length, so it fits `usize`
        Ok(Bytes::new(map[offset as usize..end as usize].to_vec()))
    }
}
