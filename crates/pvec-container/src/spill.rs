//! Spill files for the bounded-memory writer (issue pvec#235), as far as this crate goes: the container does no
//! input or output of its own, so the files are behind a trait ([`SpillStore`]) that the caller implements.
//!
//! The in-memory writer keeps every push until `finish`. A writer that has to stay inside a budget keeps its
//! pushes in files instead and sorts them with an external merge sort. This module is the part of that which
//! knows about files and nothing about PVec: [`Log`] (an append-only file of length-prefixed records) and
//! [`ExternalSorter`] (records ordered by a 128-bit key, merged from sorted runs), both written over a store of
//! numbered append-only files, which is all they ask for.
//!
//! Nothing here reaches the output: the order of equal keys is the order of arrival, and nothing ambient (a
//! clock, a pid, a file name) is ever written into a record, so the bytes a writer builds from the spill are
//! the bytes it builds from memory.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// Runs merged at once. More runs than this are merged in passes of this many, in order, so the number of
/// open runs and read buffers stays fixed whatever the budget.
const FAN_IN: usize = 64;

/// The buffer of one open run or log, in bytes.
const BUFFER: usize = 16 * 1024;

/// What the bookkeeping of one buffered record costs beyond its payload: the key, the vector header and a
/// little allocator slack.
const RECORD_OVERHEAD: usize = 64;

/// A failure of the store or of the sink, kept as the error the caller raised so that it can be got back.
#[derive(Debug)]
pub struct SpillError(Box<dyn Error + Send + Sync>);

impl SpillError {
    /// Wraps the caller's error.
    pub fn new(e: impl Error + Send + Sync + 'static) -> SpillError {
        SpillError(Box::new(e))
    }

    /// A file that does not hold what the writer put there.
    pub fn corrupt(what: &'static str) -> SpillError {
        SpillError(what.into())
    }

    /// The error the caller raised.
    pub fn into_source(self) -> Box<dyn Error + Send + Sync> {
        self.0
    }

    /// The error the caller raised, by reference.
    pub fn source_ref(&self) -> &(dyn Error + Send + Sync + 'static) {
        &*self.0
    }
}

impl fmt::Display for SpillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for SpillError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.0)
    }
}

/// A result of the store.
pub type SpillResult<T> = Result<T, SpillError>;

/// One file of a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

/// Numbered files that are written at the end and read anywhere, which is all the spill writer asks of a disk.
/// The files belong to the store: it removes them all when it is dropped, whether the writer finished, failed or
/// was never finished. It may be asked from one thread at a time.
pub trait SpillStore: fmt::Debug + Send + Sync {
    /// A new, empty file.
    fn create(&self) -> SpillResult<FileId>;
    /// Adds `bytes` at the end of `file`.
    fn append(&self, file: FileId, bytes: &[u8]) -> SpillResult<()>;
    /// Reads from `offset` into `buf` and returns how many bytes it got: fewer than `buf.len()` is allowed, and
    /// 0 means `offset` is at or past the end.
    fn read_at(&self, file: FileId, offset: u64, buf: &mut [u8]) -> SpillResult<usize>;
    /// Removes `file`.
    fn remove(&self, file: FileId) -> SpillResult<()>;
}

/// The store a writer shares with the sorters it makes.
pub type Store = Arc<dyn SpillStore>;

/// Writes to the end of a file through a buffer.
#[derive(Debug)]
pub(crate) struct Appender {
    store: Store,
    file: FileId,
    buffer: Vec<u8>,
    end: u64,
}

impl Appender {
    pub(crate) fn create(store: &Store) -> SpillResult<Appender> {
        Ok(Appender { store: Arc::clone(store), file: store.create()?, buffer: Vec::new(), end: 0 })
    }

    pub(crate) fn file(&self) -> FileId {
        self.file
    }

    /// Bytes written, buffered or not.
    pub(crate) fn end(&self) -> u64 {
        self.end
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> SpillResult<()> {
        self.end += bytes.len() as u64;
        if self.buffer.is_empty() && bytes.len() >= BUFFER {
            return self.store.append(self.file, bytes);
        }
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= BUFFER { self.flush() } else { Ok(()) }
    }

    pub(crate) fn flush(&mut self) -> SpillResult<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let out = self.store.append(self.file, &self.buffer);
        self.buffer = Vec::new();
        out
    }
}

/// Reads a file from the front through a buffer.
#[derive(Debug)]
pub(crate) struct Reader {
    store: Store,
    file: FileId,
    at: u64,
    buffer: Vec<u8>,
    used: usize,
}

impl Reader {
    pub(crate) fn open(store: &Store, file: FileId, at: u64) -> Reader {
        Reader { store: Arc::clone(store), file, at, buffer: Vec::new(), used: 0 }
    }

    /// Fills `out`, or returns `false` if the file was at its end before the first byte.
    pub(crate) fn read_exact_or_eof(&mut self, out: &mut [u8]) -> SpillResult<bool> {
        let mut done = 0;
        while done < out.len() {
            if self.used == self.buffer.len() {
                self.buffer.resize(BUFFER, 0);
                let n = self.store.read_at(self.file, self.at, &mut self.buffer)?;
                self.buffer.truncate(n);
                self.used = 0;
                self.at += n as u64;
                if n == 0 {
                    return if done == 0 {
                        Ok(false)
                    } else {
                        Err(SpillError::corrupt("a record ends early"))
                    };
                }
            }
            let take = (self.buffer.len() - self.used).min(out.len() - done);
            out[done..done + take].copy_from_slice(&self.buffer[self.used..self.used + take]);
            self.used += take;
            done += take;
        }
        Ok(true)
    }

    /// `len` bytes. A length that lies cannot reserve more than the file holds: the vector grows as bytes arrive.
    pub(crate) fn read_vec(&mut self, len: usize) -> SpillResult<Vec<u8>> {
        let mut out = Vec::with_capacity(len.min(BUFFER));
        let mut chunk = [0u8; 4096];
        while out.len() < len {
            let n = (len - out.len()).min(chunk.len());
            if !self.read_exact_or_eof(&mut chunk[..n])? {
                return Err(SpillError::corrupt("a record ends early"));
            }
            out.extend_from_slice(&chunk[..n]);
        }
        Ok(out)
    }

    /// One length-prefixed record, or `None` at a clean end of file.
    pub(crate) fn record(&mut self) -> SpillResult<Option<Vec<u8>>> {
        let mut len = [0u8; 4];
        if !self.read_exact_or_eof(&mut len)? {
            return Ok(None);
        }
        Ok(Some(self.read_vec(u32::from_le_bytes(len) as usize)?))
    }
}

/// An append-only file of records, each a `u32` length and its bytes, that can be read back in order or one
/// record at a time by the offset [`Log::append`] returned.
#[derive(Debug)]
pub struct Log {
    out: Appender,
    count: u64,
}

impl Log {
    /// A new, empty log in `store`.
    pub fn create(store: &Store) -> SpillResult<Log> {
        Ok(Log { out: Appender::create(store)?, count: 0 })
    }

    /// Appends one record and returns its offset.
    pub fn append(&mut self, bytes: &[u8]) -> SpillResult<u64> {
        let len =
            u32::try_from(bytes.len()).map_err(|_| SpillError::corrupt("a record is longer than 4 GiB"))?;
        let at = self.out.end();
        self.out.write(&len.to_le_bytes())?;
        self.out.write(bytes)?;
        self.count += 1;
        Ok(at)
    }

    /// Records appended so far.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The record at `offset`.
    pub fn read_at(&mut self, offset: u64) -> SpillResult<Vec<u8>> {
        self.out.flush()?;
        let mut r = Reader::open(&self.out.store, self.out.file(), offset);
        r.record()?.ok_or_else(|| SpillError::corrupt("no record at that offset"))
    }

    /// Every record in the order it was appended.
    pub fn records(&mut self) -> SpillResult<LogRecords> {
        self.out.flush()?;
        Ok(LogRecords { reader: Reader::open(&self.out.store, self.out.file(), 0), left: self.count })
    }
}

/// The records of a [`Log`], in order.
#[derive(Debug)]
pub struct LogRecords {
    reader: Reader,
    left: u64,
}

impl Iterator for LogRecords {
    type Item = SpillResult<Vec<u8>>;

    fn next(&mut self) -> Option<SpillResult<Vec<u8>>> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        match self.reader.record() {
            Ok(Some(r)) => Some(Ok(r)),
            Ok(None) => Some(Err(SpillError::corrupt("the log ends early"))),
            Err(e) => Some(Err(e)),
        }
    }
}

/// Records ordered by a 128-bit key. Equal keys keep the order they were pushed in.
///
/// Up to `budget` bytes of records are held and sorted in memory; past that the buffer is written out as a
/// sorted run and a new one starts. [`ExternalSorter::finish`] merges the runs. With a budget big enough for
/// everything no file is made.
#[derive(Debug)]
pub struct ExternalSorter {
    store: Store,
    budget: usize,
    buffer: Vec<(u128, Vec<u8>)>,
    held: usize,
    runs: Vec<FileId>,
    count: u64,
}

impl ExternalSorter {
    /// A sorter that holds at most about `budget` bytes of records.
    pub fn new(store: &Store, budget: usize) -> ExternalSorter {
        ExternalSorter {
            store: Arc::clone(store),
            budget,
            buffer: Vec::new(),
            held: 0,
            runs: Vec::new(),
            count: 0,
        }
    }

    /// Adds a record.
    pub fn push(&mut self, key: u128, payload: Vec<u8>) -> SpillResult<()> {
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

    fn spill(&mut self) -> SpillResult<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.buffer.sort_by_key(|r| r.0);
        let mut run = Appender::create(&self.store)?;
        for (key, payload) in self.buffer.drain(..) {
            write_run_record(&mut run, key, &payload)?;
        }
        run.flush()?;
        self.runs.push(run.file());
        self.held = 0;
        // release the buffer's own allocation too: a tiny budget must not keep a big vector alive
        self.buffer = Vec::new();
        Ok(())
    }

    /// Ends the pushes and returns the records in key order.
    pub fn finish(mut self) -> SpillResult<Sorted> {
        if self.runs.is_empty() {
            self.buffer.sort_by_key(|r| r.0);
            return Ok(Sorted(Inner::Memory(std::mem::take(&mut self.buffer).into_iter())));
        }
        self.spill()?;
        while self.runs.len() > FAN_IN {
            let mut merged = Vec::with_capacity(self.runs.len().div_ceil(FAN_IN));
            for group in self.runs.chunks(FAN_IN) {
                let mut out = Appender::create(&self.store)?;
                for rec in Merge::open(&self.store, group)? {
                    let (key, payload) = rec?;
                    write_run_record(&mut out, key, &payload)?;
                }
                out.flush()?;
                for old in group {
                    self.store.remove(*old)?;
                }
                merged.push(out.file());
            }
            self.runs = merged;
        }
        Ok(Sorted(Inner::Merge(Merge::open(&self.store, &self.runs)?)))
    }
}

fn write_run_record(w: &mut Appender, key: u128, payload: &[u8]) -> SpillResult<()> {
    let len =
        u32::try_from(payload.len()).map_err(|_| SpillError::corrupt("a record is longer than 4 GiB"))?;
    w.write(&key.to_le_bytes())?;
    w.write(&len.to_le_bytes())?;
    w.write(payload)
}

fn read_run_record(r: &mut Reader) -> SpillResult<Option<(u128, Vec<u8>)>> {
    let mut key = [0u8; 16];
    if !r.read_exact_or_eof(&mut key)? {
        return Ok(None);
    }
    match r.record()? {
        Some(payload) => Ok(Some((u128::from_le_bytes(key), payload))),
        None => Err(SpillError::corrupt("a run ends early")),
    }
}

/// A k-way merge of sorted runs. A tie goes to the earlier run, which holds the earlier pushes.
#[derive(Debug)]
struct Merge {
    runs: Vec<Reader>,
    heads: Vec<Option<Vec<u8>>>,
    heap: BinaryHeap<Reverse<(u128, usize)>>,
}

impl Merge {
    fn open(store: &Store, files: &[FileId]) -> SpillResult<Merge> {
        let mut m = Merge { runs: Vec::new(), heads: Vec::new(), heap: BinaryHeap::new() };
        for (i, f) in files.iter().enumerate() {
            m.runs.push(Reader::open(store, *f, 0));
            m.heads.push(None);
            m.refill(i)?;
        }
        Ok(m)
    }

    fn refill(&mut self, i: usize) -> SpillResult<()> {
        if let Some((key, payload)) = read_run_record(&mut self.runs[i])? {
            self.heads[i] = Some(payload);
            self.heap.push(Reverse((key, i)));
        }
        Ok(())
    }
}

impl Iterator for Merge {
    type Item = SpillResult<(u128, Vec<u8>)>;

    fn next(&mut self) -> Option<SpillResult<(u128, Vec<u8>)>> {
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
    type Item = SpillResult<(u128, Vec<u8>)>;

    fn next(&mut self) -> Option<SpillResult<(u128, Vec<u8>)>> {
        match &mut self.0 {
            Inner::Memory(it) => it.next().map(Ok),
            Inner::Merge(m) => m.next(),
        }
    }
}
