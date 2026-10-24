//! File sources for the PVec reader (issue P4.5).
//!
//! `pvec-container` is an API crate: it opens no file, socket or process, which is checked on every
//! build (`crates/pvec-checks/tests/crate_boundaries.rs`). Reading a local file needs `std::fs`, so the
//! two sources that do are here, and implement the traits that `pvec_container::source` defines. What
//! a file returns is untrusted like any other bytes: the reader holds it to the same checks, and a file
//! that is shorter than it was when it was opened is a `SourceError::Short`, not a short decode.
//!
//! There is no memory-mapped source, because mapping needs `unsafe` and this workspace allows none
//! outside `pvec-container`'s `aligned.rs`. A caller that maps a file hands the mapped slice to
//! `pvec_container::source::MemorySource`.
#![forbid(unsafe_code)]

use pvec_container::source::{BlockSource, Bytes, ReadAt, SourceError};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

fn io(e: std::io::Error) -> SourceError {
    SourceError::Io(e.to_string())
}

fn check(offset: u64, len: u32, source_len: u64) -> Result<(), SourceError> {
    match offset.checked_add(u64::from(len)) {
        Some(end) if end <= source_len => Ok(()),
        _ => Err(SourceError::OutOfRange { offset, len: u64::from(len), source_len }),
    }
}

/// A local file, read with a seek and a read under a lock, so it works wherever `std::fs` does.
#[derive(Debug)]
pub struct FileSource {
    file: Mutex<File>,
    len: u64,
}

impl FileSource {
    /// Opens `path` read-only.
    pub fn open(path: impl AsRef<Path>) -> Result<FileSource, SourceError> {
        let file = File::open(path).map_err(io)?;
        let len = file.metadata().map_err(io)?.len();
        Ok(FileSource { file: Mutex::new(file), len })
    }
}

impl BlockSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        check(offset, len, self.len)?;
        let mut buf = vec![0u8; len as usize];
        let mut file = self.file.lock().map_err(|_| SourceError::Io("file lock poisoned".into()))?;
        file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut done = 0usize;
        while done < buf.len() {
            let n = file.read(&mut buf[done..]).map_err(io)?;
            if n == 0 {
                return Err(SourceError::Short { wanted: u64::from(len), got: done as u64 });
            }
            done += n;
        }
        Ok(Bytes::new(buf))
    }
}

/// A file with a positional read (`pread` on Unix), for use with `pvec_container::source::ReadAtSource`.
#[derive(Debug)]
pub struct PositionalFile {
    file: File,
}

impl PositionalFile {
    /// Opens `path` read-only. The length is the file's, for `ReadAtSource::new`.
    pub fn open(path: impl AsRef<Path>) -> Result<(PositionalFile, u64), SourceError> {
        let file = File::open(path).map_err(io)?;
        let len = file.metadata().map_err(io)?.len();
        Ok((PositionalFile { file }, len))
    }
}

impl ReadAt for PositionalFile {
    #[cfg(unix)]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        std::os::unix::fs::FileExt::read_at(&self.file, buf, offset).map_err(io)
    }

    #[cfg(not(unix))]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        std::os::windows::fs::FileExt::seek_read(&self.file, buf, offset).map_err(io)
    }
}
