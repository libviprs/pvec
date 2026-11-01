//! The files of the bounded-memory writer (issue pvec#235): a directory the writer makes under a root the caller
//! names, numbered append-only files in it, and the removal of the whole directory when the store is dropped,
//! whether the writer finished, failed or was never finished. `pvec-container` does no file access of its own, so
//! this is where the disk is. The container's tests include this file as it is.
#![allow(dead_code)]

use pvec_container::spill::{FileId, SpillError, SpillResult, SpillStore};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static STORES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    writer: File,
    reader: Option<File>,
}

#[derive(Debug, Default)]
struct State {
    next: u32,
    files: HashMap<u32, Entry>,
}

/// A directory of spill files.
#[derive(Debug)]
pub(crate) struct FsStore {
    dir: PathBuf,
    state: Mutex<State>,
}

impl FsStore {
    /// Makes a fresh directory in `root`. A `root` that does not exist is an error (`NotFound`), never something
    /// to create: the caller decides where the bytes go.
    pub(crate) fn create(root: &Path) -> io::Result<FsStore> {
        let base = std::process::id();
        loop {
            let n = STORES.fetch_add(1, Ordering::Relaxed);
            let dir = root.join(format!("pvec-spill-{base}-{n}"));
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(FsStore { dir, state: Mutex::new(State::default()) }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// The directory the files are in.
    pub(crate) fn path(&self) -> &Path {
        &self.dir
    }

    fn state(&self) -> SpillResult<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| SpillError::corrupt("the spill store was poisoned"))
    }
}

impl SpillStore for FsStore {
    fn create(&self) -> SpillResult<FileId> {
        let mut st = self.state()?;
        let id = st.next;
        let path = self.dir.join(format!("f{id}"));
        let writer =
            OpenOptions::new().append(true).create_new(true).open(&path).map_err(SpillError::new)?;
        st.next += 1;
        st.files.insert(id, Entry { path, writer, reader: None });
        Ok(FileId(id))
    }

    fn append(&self, file: FileId, bytes: &[u8]) -> SpillResult<()> {
        let mut st = self.state()?;
        let e = st.files.get_mut(&file.0).ok_or_else(|| SpillError::corrupt("a spill file that is not there"))?;
        e.writer.write_all(bytes).map_err(SpillError::new)
    }

    fn read_at(&self, file: FileId, offset: u64, buf: &mut [u8]) -> SpillResult<usize> {
        let mut st = self.state()?;
        let e = st.files.get_mut(&file.0).ok_or_else(|| SpillError::corrupt("a spill file that is not there"))?;
        if e.reader.is_none() {
            e.reader = Some(File::open(&e.path).map_err(SpillError::new)?);
        }
        let Some(r) = e.reader.as_mut() else { return Err(SpillError::corrupt("no reader")) };
        r.seek(SeekFrom::Start(offset)).map_err(SpillError::new)?;
        loop {
            match r.read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(SpillError::new(e)),
            }
        }
    }

    fn remove(&self, file: FileId) -> SpillResult<()> {
        let mut st = self.state()?;
        if let Some(e) = st.files.remove(&file.0) {
            drop(e.reader);
            drop(e.writer);
            fs::remove_file(&e.path).map_err(SpillError::new)?;
        }
        Ok(())
    }
}

impl Drop for FsStore {
    fn drop(&mut self) {
        // best effort: a directory that cannot be removed is a leak, and a destructor has nobody to tell
        let _ = fs::remove_dir_all(&self.dir);
    }
}
