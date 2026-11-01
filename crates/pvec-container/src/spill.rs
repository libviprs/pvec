//! Spill files for the bounded-memory writer (issue pvec#235).
//!
//! The in-memory writer keeps every push until `finish`. A writer that has to stay inside a budget keeps
//! its pushes in files instead, under a directory it owns beneath a root the caller names, and sorts them
//! with an external merge sort. This module is the part of that which knows about files and nothing about
//! PVec: [`SpillDir`] (the directory, removed when the last holder lets go), [`Log`] (an append-only file
//! of length-prefixed records) and [`ExternalSorter`] (records ordered by a 128-bit key, merged from sorted
//! runs).
//!
//! Nothing here reaches the output: the order of equal keys is the order of arrival, and nothing ambient
//! (a clock, a pid, a file name) is ever written into a record, so the bytes a writer builds from the spill
//! are the bytes it builds from memory.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Runs merged at once. More runs than this are merged in passes of this many, in order, so the number of
/// open files and read buffers stays fixed whatever the budget.
const FAN_IN: usize = 64;

/// The buffer of one open run, in bytes.
const RUN_BUFFER: usize = 16 * 1024;

/// What the bookkeeping of one buffered record costs beyond its payload: the key, the vector header and a
/// little allocator slack.
const RECORD_OVERHEAD: usize = 64;

static DIRS: AtomicU64 = AtomicU64::new(0);

/// A directory under the caller's root that holds the files of one writer. It is created empty, only inside
/// a root that already exists, and removed with everything in it when the last [`Arc`] to it is dropped,
/// whether the writer finished, failed or was never finished.
#[derive(Debug)]
pub struct SpillDir {
    path: PathBuf,
    files: AtomicU64,
}

impl SpillDir {
    /// Makes a fresh directory in `root`. A `root` that does not exist is an error (`NotFound`), never
    /// something to create: the caller decides where the bytes go.
    pub fn create(root: &Path) -> io::Result<Arc<SpillDir>> {
        let base = std::process::id();
        loop {
            let n = DIRS.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("pvec-spill-{base}-{n}"));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Arc::new(SpillDir { path, files: AtomicU64::new(0) })),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A path for a new file in the directory, not used before.
    pub fn next_path(&self) -> PathBuf {
        self.path.join(format!("f{}", self.files.fetch_add(1, Ordering::Relaxed)))
    }
}

impl Drop for SpillDir {
    fn drop(&mut self) {
        // best effort: a directory that cannot be removed is a leak, and a destructor has nobody to tell
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// An append-only file of records, each a `u32` length and its bytes, that can be read back in order or
/// one record at a time by the offset [`Log::append`] returned.
#[derive(Debug)]
pub struct Log {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    reader: Option<File>,
    end: u64,
    count: u64,
}

impl Log {
    /// A new, empty log in `dir`.
    pub fn create(dir: &SpillDir) -> io::Result<Log> {
        let path = dir.next_path();
        let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
        Ok(Log { path, writer: Some(BufWriter::with_capacity(RUN_BUFFER * 4, file)), reader: None, end: 0, count: 0 })
    }

    /// Appends one record and returns its offset.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<u64> {
        let len = u32::try_from(bytes.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record too long"))?;
        let at = self.end;
        let w = self.writer.as_mut().ok_or_else(|| io::Error::other("the log is closed"))?;
        w.write_all(&len.to_le_bytes())?;
        w.write_all(bytes)?;
        self.end += 4 + u64::from(len);
        self.count += 1;
        Ok(at)
    }

    /// Records appended so far.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Bytes appended so far, length prefixes included.
    pub fn bytes(&self) -> u64 {
        self.end
    }

    /// Writes what is buffered to the file, so that it can be read.
    pub fn flush(&mut self) -> io::Result<()> {
        match self.writer.as_mut() {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }

    /// The record at `offset`.
    pub fn read_at(&mut self, offset: u64) -> io::Result<Vec<u8>> {
        self.flush()?;
        if self.reader.is_none() {
            self.reader = Some(File::open(&self.path)?);
        }
        let f = self.reader.as_mut().ok_or_else(|| io::Error::other("the log has no reader"))?;
        f.seek(SeekFrom::Start(offset))?;
        read_record(f)?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no record at that offset"))
    }

    /// Every record in the order it was appended.
    pub fn records(&mut self) -> io::Result<LogRecords> {
        self.flush()?;
        let f = File::open(&self.path)?;
        Ok(LogRecords { reader: BufReader::with_capacity(RUN_BUFFER, f), left: self.count })
    }
}

/// The records of a [`Log`], in order.
#[derive(Debug)]
pub struct LogRecords {
    reader: BufReader<File>,
    left: u64,
}

impl Iterator for LogRecords {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<io::Result<Vec<u8>>> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        match read_record(&mut self.reader) {
            Ok(Some(r)) => Some(Ok(r)),
            Ok(None) => Some(Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the log ends early"))),
            Err(e) => Some(Err(e)),
        }
    }
}

/// One length-prefixed record, or `None` at a clean end of file.
fn read_record(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    if !read_exact_or_eof(r, &mut len)? {
        return Ok(None);
    }
    let len = u32::from_le_bytes(len) as usize;
    // `take` and `read_to_end` grow as bytes arrive, so a length that lies cannot reserve more than the file holds
    let mut out = Vec::new();
    r.take(len as u64).read_to_end(&mut out)?;
    if out.len() != len {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "a record ends early"));
    }
    Ok(Some(out))
}

/// Fills `buf`, or returns `false` if the reader was at its end before the first byte.
fn read_exact_or_eof(r: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    let mut at = 0;
    while at < buf.len() {
        match r.read(&mut buf[at..]) {
            Ok(0) if at == 0 => return Ok(false),
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "a record ends early")),
            Ok(n) => at += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Records ordered by a 128-bit key. Equal keys keep the order they were pushed in.
///
/// Up to `budget` bytes of records are held and sorted in memory; past that the buffer is written out as a
/// sorted run and a new one starts. [`ExternalSorter::finish`] merges the runs. With a budget big enough for
/// everything no file is made.
#[derive(Debug)]
pub struct ExternalSorter {
    dir: Arc<SpillDir>,
    budget: usize,
    buffer: Vec<(u128, Vec<u8>)>,
    held: usize,
    runs: Vec<PathBuf>,
    count: u64,
}

impl ExternalSorter {
    /// A sorter that holds at most about `budget` bytes of records.
    pub fn new(dir: Arc<SpillDir>, budget: usize) -> ExternalSorter {
        ExternalSorter { dir, budget, buffer: Vec::new(), held: 0, runs: Vec::new(), count: 0 }
    }

    /// Adds a record.
    pub fn push(&mut self, key: u128, payload: Vec<u8>) -> io::Result<()> {
        self.held += payload.len() + RECORD_OVERHEAD;
        self.count += 1;
        self.buffer.push((key, payload));
        if self.held >= self.budget { self.spill() } else { Ok(()) }
    }

    /// Records pushed so far.
    pub fn len(&self) -> u64 {
        self.count
    }

    /// Whether none was pushed.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn spill(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.buffer.sort_by_key(|r| r.0);
        let path = self.dir.next_path();
        let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
        let mut w = BufWriter::with_capacity(RUN_BUFFER, file);
        for (key, payload) in self.buffer.drain(..) {
            write_run_record(&mut w, key, &payload)?;
        }
        w.flush()?;
        self.runs.push(path);
        self.held = 0;
        // release the buffer's own allocation too: a tiny budget must not keep a big vector alive
        self.buffer = Vec::new();
        Ok(())
    }

    /// Ends the pushes and returns the records in key order.
    pub fn finish(mut self) -> io::Result<Sorted> {
        if self.runs.is_empty() {
            self.buffer.sort_by_key(|r| r.0);
            return Ok(Sorted(Inner::Memory(std::mem::take(&mut self.buffer).into_iter())));
        }
        self.spill()?;
        while self.runs.len() > FAN_IN {
            let mut merged = Vec::with_capacity(self.runs.len().div_ceil(FAN_IN));
            for group in self.runs.chunks(FAN_IN) {
                let path = self.dir.next_path();
                let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
                let mut w = BufWriter::with_capacity(RUN_BUFFER, file);
                for rec in Merge::open(group)? {
                    let (key, payload) = rec?;
                    write_run_record(&mut w, key, &payload)?;
                }
                w.flush()?;
                for old in group {
                    fs::remove_file(old)?;
                }
                merged.push(path);
            }
            self.runs = merged;
        }
        Ok(Sorted(Inner::Merge(Merge::open(&self.runs)?)))
    }
}

fn write_run_record(w: &mut impl Write, key: u128, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record too long"))?;
    w.write_all(&key.to_le_bytes())?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)
}

fn read_run_record(r: &mut impl Read) -> io::Result<Option<(u128, Vec<u8>)>> {
    let mut key = [0u8; 16];
    if !read_exact_or_eof(r, &mut key)? {
        return Ok(None);
    }
    match read_record(r)? {
        Some(payload) => Ok(Some((u128::from_le_bytes(key), payload))),
        None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "a run ends early")),
    }
}

/// A k-way merge of sorted runs. A tie goes to the earlier run, which holds the earlier pushes.
#[derive(Debug)]
struct Merge {
    runs: Vec<BufReader<File>>,
    heads: Vec<Option<Vec<u8>>>,
    heap: BinaryHeap<Reverse<(u128, usize)>>,
}

impl Merge {
    fn open(paths: &[PathBuf]) -> io::Result<Merge> {
        let mut m = Merge { runs: Vec::new(), heads: Vec::new(), heap: BinaryHeap::new() };
        for (i, p) in paths.iter().enumerate() {
            m.runs.push(BufReader::with_capacity(RUN_BUFFER, File::open(p)?));
            m.heads.push(None);
            m.refill(i)?;
        }
        Ok(m)
    }

    fn refill(&mut self, i: usize) -> io::Result<()> {
        if let Some((key, payload)) = read_run_record(&mut self.runs[i])? {
            self.heads[i] = Some(payload);
            self.heap.push(Reverse((key, i)));
        }
        Ok(())
    }
}

impl Iterator for Merge {
    type Item = io::Result<(u128, Vec<u8>)>;

    fn next(&mut self) -> Option<io::Result<(u128, Vec<u8>)>> {
        let Reverse((key, i)) = self.heap.pop()?;
        let payload = self.heads[i].take().unwrap_or_default();
        match self.refill(i) {
            Ok(()) => Some(Ok((key, payload))),
            Err(e) => Some(Err(e)),
        }
    }
}

#[derive(Debug)]
enum Inner {
    Memory(std::vec::IntoIter<(u128, Vec<u8>)>),
    Merge(Merge),
}

/// The records of an [`ExternalSorter`] in key order.
#[derive(Debug)]
pub struct Sorted(Inner);

impl Iterator for Sorted {
    type Item = io::Result<(u128, Vec<u8>)>;

    fn next(&mut self) -> Option<io::Result<(u128, Vec<u8>)>> {
        match &mut self.0 {
            Inner::Memory(it) => it.next().map(Ok),
            Inner::Merge(m) => m.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pvec-spill-unit-{}-{tag}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *seed >> 33
    }

    #[test]
    fn the_directory_is_made_in_an_existing_root_only_and_goes_with_its_last_holder() {
        let r = root("dir");
        let missing = r.join("nothing-here");
        assert_eq!(SpillDir::create(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(!missing.exists());
        let d = SpillDir::create(&r).unwrap();
        let other = Arc::clone(&d);
        let path = d.path().to_path_buf();
        assert!(path.starts_with(&r) && path.is_dir());
        drop(d);
        assert!(path.is_dir(), "a second holder keeps it");
        drop(other);
        assert!(!path.exists());
        fs::remove_dir_all(&r).unwrap();
    }

    #[test]
    fn a_log_reads_back_in_order_and_by_offset() {
        let r = root("log");
        let d = SpillDir::create(&r).unwrap();
        let mut log = Log::create(&d).unwrap();
        let mut at = Vec::new();
        for i in 0..1000u32 {
            at.push(log.append(&vec![i as u8; (i % 37) as usize]).unwrap());
        }
        assert_eq!(log.count(), 1000);
        assert_eq!(log.read_at(at[500]).unwrap(), vec![500u32 as u8; 500 % 37]);
        let all: Vec<Vec<u8>> = log.records().unwrap().map(Result::unwrap).collect();
        assert_eq!(all.len(), 1000);
        assert_eq!(all[999], vec![999u32 as u8; 999 % 37]);
        drop(log);
        drop(d);
        fs::remove_dir_all(&r).unwrap();
    }

    #[test]
    fn the_sorter_agrees_with_a_stable_sort_at_every_budget_and_pushes_equal_keys_in_arrival_order() {
        let r = root("sort");
        for budget in [0usize, 1, 100, 5000, 1 << 20] {
            for n in [0usize, 1, 7, 500, 3000] {
                let d = SpillDir::create(&r).unwrap();
                let mut seed = (n as u64) << 8 | budget as u64 & 0xff;
                let mut sorter = ExternalSorter::new(Arc::clone(&d), budget);
                let mut model = Vec::new();
                for i in 0..n {
                    let key = u128::from(lcg(&mut seed) % 50) << 70 | u128::from(lcg(&mut seed) % 3);
                    let payload = (i as u32).to_le_bytes().repeat(1 + (lcg(&mut seed) % 5) as usize);
                    model.push((key, payload.clone()));
                    sorter.push(key, payload).unwrap();
                }
                model.sort_by_key(|m| m.0);
                let got: Vec<(u128, Vec<u8>)> = sorter.finish().unwrap().map(Result::unwrap).collect();
                assert_eq!(got, model, "budget {budget}, {n} records");
                drop(d);
            }
        }
        assert_eq!(fs::read_dir(&r).unwrap().count(), 0, "every spill directory is gone");
        fs::remove_dir_all(&r).unwrap();
    }

    #[test]
    fn more_runs_than_the_fan_in_are_merged_in_passes_and_stay_stable() {
        let r = root("passes");
        let d = SpillDir::create(&r).unwrap();
        let mut sorter = ExternalSorter::new(Arc::clone(&d), 1);
        let mut model = Vec::new();
        for i in 0..(FAN_IN * 3 + 5) as u32 {
            let key = u128::from(i % 4);
            model.push((key, i.to_le_bytes().to_vec()));
            sorter.push(key, i.to_le_bytes().to_vec()).unwrap();
        }
        model.sort_by_key(|m| m.0);
        let got: Vec<_> = sorter.finish().unwrap().map(Result::unwrap).collect();
        assert_eq!(got, model);
        drop(d);
        fs::remove_dir_all(&r).unwrap();
    }

    #[test]
    fn a_truncated_run_is_an_error_and_not_a_short_result() {
        let r = root("trunc");
        let d = SpillDir::create(&r).unwrap();
        let mut sorter = ExternalSorter::new(Arc::clone(&d), 1);
        for i in 0..10u32 {
            sorter.push(u128::from(i), vec![7; 40]).unwrap();
        }
        let path = fs::read_dir(d.path()).unwrap().next().unwrap().unwrap().path();
        let len = fs::metadata(&path).unwrap().len();
        OpenOptions::new().write(true).open(&path).unwrap().set_len(len - 5).unwrap();
        let outcome: io::Result<Vec<_>> = sorter.finish().and_then(|s| s.collect());
        assert!(outcome.is_err());
        drop(d);
        fs::remove_dir_all(&r).unwrap();
    }
}
