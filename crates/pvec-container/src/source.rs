//! Where the bytes of an artifact come from (issue P4.5).
//!
//! A `.pvec` file arrives from object storage over HTTP ranges, from a browser cache, from a local file
//! or from memory a caller has mapped. The reader is written against [`BlockSource`] (and its async twin
//! [`AsyncBlockSource`]) and treats whatever a source returns as untrusted: a source that answers with
//! fewer bytes than were asked for is an error here, and every offset and length the reader asks for was
//! checked against the artifact length first.
//!
//! What is here: [`MemorySource`] (bytes already in memory, which is also how a caller's own memory map
//! is used: the mapped bytes go through the same checks as any others), [`ReadAtSource`] (anything with
//! a positional read) and [`HttpRangeSource`] (an async HTTP range client over a caller-supplied
//! transport, so this crate opens no socket and needs no runtime). [`Recording`] wraps any of them and
//! logs what was asked, which is how the tests count requests.
//!
//! This is an API crate, and an API crate does no file, network or process access
//! (`crates/pvec-checks/tests/crate_boundaries.rs`), so the sources that touch `std::fs` (a local file,
//! and a `ReadAt` over one) are in the `pvec-sources` crate, which depends on this one.
//!
//! There is no `MmapSource` here: mapping a file needs `unsafe`, and the only `unsafe` this crate allows is
//! the confined casts in `blocks/aligned.rs`. The mapped source is the `pvec-mmap` crate (ADR 0016), which
//! holds the one other `unsafe` block of the workspace. A caller who maps a file themselves passes the
//! mapped slice to [`MemorySource`] and gets the same checks.

use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

/// Bytes returned by a source: a shared buffer and the part of it that was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bytes {
    data: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl Bytes {
    /// Wraps a buffer, without copying it.
    pub fn new(data: Vec<u8>) -> Bytes {
        let end = data.len();
        Bytes { data: Arc::new(data), start: 0, end }
    }

    /// The sub-range `start..end` of these bytes, sharing the buffer, or `None` when it is not inside.
    pub fn slice(&self, start: usize, end: usize) -> Option<Bytes> {
        if start > end || end > self.len() {
            return None;
        }
        Some(Bytes { data: Arc::clone(&self.data), start: self.start + start, end: self.start + end })
    }

    /// The bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.data[self.start..self.end]
    }
}

impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Bytes {
        Bytes::new(v)
    }
}

/// Why a source could not supply a range.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceError {
    /// The range is not inside the source.
    OutOfRange {
        /// First byte asked for.
        offset: u64,
        /// Bytes asked for.
        len: u64,
        /// Length of the source.
        source_len: u64,
    },
    /// The source returned a different number of bytes than were asked for. A short answer is never
    /// decoded.
    Short {
        /// Bytes asked for.
        wanted: u64,
        /// Bytes returned.
        got: u64,
    },
    /// The server did not honour the range (it answered with a status or a range other than the one
    /// asked for).
    RangeNotHonoured {
        /// What it answered with.
        detail: String,
    },
    /// The object changed since the source first read from it: the server answered an `If-Range` with
    /// the whole new object, a range came back with another validator, or the `Content-Range` total is
    /// not the length the source was opened with. Nothing from the new object is handed to the decoder;
    /// the caller reopens the file.
    Changed {
        /// What showed the change.
        detail: String,
    },
    /// The transport or the file failed.
    Io(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::OutOfRange { offset, len, source_len } => {
                write!(f, "range {offset}+{len} is outside a source of {source_len} bytes")
            }
            SourceError::Short { wanted, got } => write!(f, "asked for {wanted} bytes and got {got}"),
            SourceError::RangeNotHonoured { detail } => write!(f, "the range was not honoured: {detail}"),
            SourceError::Changed { detail } => write!(f, "the object changed since it was opened: {detail}"),
            SourceError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// A synchronous source of byte ranges.
pub trait BlockSource {
    /// The length of the artifact in bytes.
    fn len(&self) -> u64;

    /// Whether the artifact is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Exactly `len` bytes from `offset`. A source that cannot supply all of them returns an error, not
    /// the part it has.
    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError>;
}

/// An asynchronous source of byte ranges: the same contract as [`BlockSource`].
pub trait AsyncBlockSource {
    /// The length of the artifact in bytes.
    fn len(&self) -> u64;

    /// Whether the artifact is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Exactly `len` bytes from `offset`.
    fn read_range(&self, offset: u64, len: u32) -> impl Future<Output = Result<Bytes, SourceError>>;
}

fn check_range(offset: u64, len: u32, source_len: u64) -> Result<(), SourceError> {
    match offset.checked_add(u64::from(len)) {
        Some(end) if end <= source_len => Ok(()),
        _ => Err(SourceError::OutOfRange { offset, len: u64::from(len), source_len }),
    }
}

/// Bytes in memory: a `Vec`, a slice, or a caller's memory map. Every range is a copy, so what the
/// reader decodes is never the caller's own buffer.
#[derive(Clone, Debug)]
pub struct MemorySource<T: AsRef<[u8]>> {
    data: T,
}

impl<T: AsRef<[u8]>> MemorySource<T> {
    /// A source over `data`.
    pub fn new(data: T) -> MemorySource<T> {
        MemorySource { data }
    }
}

impl<T: AsRef<[u8]>> BlockSource for MemorySource<T> {
    fn len(&self) -> u64 {
        self.data.as_ref().len() as u64
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        let all = self.data.as_ref();
        check_range(offset, len, all.len() as u64)?;
        // in range by the check above, and `offset + len` fits `usize` because it is at most `all.len()`
        let start = offset as usize;
        Ok(Bytes::new(all[start..start + len as usize].to_vec()))
    }
}

impl<T: AsRef<[u8]>> AsyncBlockSource for MemorySource<T> {
    fn len(&self) -> u64 {
        BlockSource::len(self)
    }

    async fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        BlockSource::read_range(self, offset, len)
    }
}

/// Anything that can read at a position, like `pread`.
pub trait ReadAt {
    /// Reads up to `buf.len()` bytes at `offset`, returning how many (0 at the end).
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError>;
}

impl ReadAt for [u8] {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        let Ok(start) = usize::try_from(offset) else { return Ok(0) };
        let Some(rest) = self.get(start..) else { return Ok(0) };
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        Ok(n)
    }
}

impl<T: ReadAt + ?Sized> ReadAt for Arc<T> {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        (**self).read_at(buf, offset)
    }
}

/// A [`ReadAt`] with a known length.
#[derive(Debug)]
pub struct ReadAtSource<R: ReadAt> {
    inner: R,
    len: u64,
}

impl<R: ReadAt> ReadAtSource<R> {
    /// A source over `inner`, which is `len` bytes long.
    pub fn new(inner: R, len: u64) -> ReadAtSource<R> {
        ReadAtSource { inner, len }
    }
}

impl<R: ReadAt> BlockSource for ReadAtSource<R> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        check_range(offset, len, self.len)?;
        // `len` is a `u32` the caller already bounded; the buffer is the size of the range asked for
        let mut buf = vec![0u8; len as usize];
        let mut done = 0usize;
        while done < buf.len() {
            let n = self.inner.read_at(&mut buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(SourceError::Short { wanted: u64::from(len), got: done as u64 });
            }
            done += n;
        }
        Ok(Bytes::new(buf))
    }
}

impl<R: ReadAt> AsyncBlockSource for ReadAtSource<R> {
    fn len(&self) -> u64 {
        self.len
    }

    async fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        BlockSource::read_range(self, offset, len)
    }
}

/// An HTTP range request: both ends inclusive, as in the `Range: bytes=start-end` header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpRange {
    /// First byte.
    pub start: u64,
    /// Last byte, inclusive.
    pub end: u64,
}

/// What the transport got back.
#[derive(Clone, Debug)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The `Content-Range` header as `(first, last, total)`, when the server sent one.
    pub content_range: Option<(u64, u64, Option<u64>)>,
    /// The body.
    pub body: Vec<u8>,
}

/// The one thing an HTTP client has to do for [`HttpRangeSource`]: fetch a range.
///
/// A transport must not read more of a response than the range it was asked for (`end - start + 1`
/// bytes): the body it returns is allocated before this crate sees it, so a server that sends more than
/// was asked for is the transport's to cut off, not the reader's.
pub trait HttpTransport {
    /// Sends `GET` with the range header and returns the response.
    fn fetch(&self, range: HttpRange) -> impl Future<Output = Result<HttpResponse, String>>;

    /// Sends `GET` with the range header and, when `if_range` is `Some`, an `If-Range` header with that
    /// value, and returns the response with the validator it carried: its strong `ETag` as sent (quotes
    /// included), else its `Last-Modified`, else `None`. On a `200` the transport may drop the body.
    ///
    /// The default sends no `If-Range` and reports no validator, so a transport that only implements
    /// [`HttpTransport::fetch`] reads as it always did, without the change check.
    fn fetch_if_range(
        &self,
        range: HttpRange,
        if_range: Option<&str>,
    ) -> impl Future<Output = Result<(HttpResponse, Option<String>), String>> {
        let _ = if_range;
        async move { self.fetch(range).await.map(|r| (r, None)) }
    }
}

/// An artifact behind HTTP range requests. The transport is the caller's (a `fetch` in a browser, a
/// client over a socket in a service), which keeps this crate free of both.
///
/// A response is accepted only if it is a `206` whose body is exactly the bytes asked for. A `200` (the
/// server ignored the range and sent the whole object), a `206` for another range and a body of another
/// length are errors: a short answer is never handed to the decoder.
///
/// The source also holds the object to the version it first read (issue #264). The first strong validator
/// a response carries is kept and sent as `If-Range` with every later range, so a server answers a
/// rewritten object with a `200`, which is [`SourceError::Changed`]. A `206` under another validator (a
/// server that ignores `If-Range`) and a `Content-Range` total other than `len` are `Changed` as well. A
/// weak `ETag` is never sent, because `If-Range` takes a strong validator only (RFC 9110 13.1.5).
#[derive(Debug)]
pub struct HttpRangeSource<T: HttpTransport> {
    transport: T,
    len: u64,
    validator: Mutex<Option<String>>,
}

impl<T: HttpTransport> HttpRangeSource<T> {
    /// A source over `transport` for an object of `len` bytes (from a `HEAD`, or a suffix range).
    pub fn new(transport: T, len: u64) -> HttpRangeSource<T> {
        HttpRangeSource { transport, len, validator: Mutex::new(None) }
    }
}

impl<T: HttpTransport> AsyncBlockSource for HttpRangeSource<T> {
    fn len(&self) -> u64 {
        self.len
    }

    async fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        check_range(offset, len, self.len)?;
        if len == 0 {
            return Ok(Bytes::new(Vec::new()));
        }
        let range = HttpRange { start: offset, end: offset + u64::from(len) - 1 };
        let lock = || self.validator.lock().map_err(|_| SourceError::Io("validator lock poisoned".into()));
        let sent = lock()?.clone();
        let (response, validator) =
            self.transport.fetch_if_range(range, sent.as_deref()).await.map_err(SourceError::Io)?;
        if response.status == 200 && sent.is_some() {
            return Err(SourceError::Changed {
                detail: format!(
                    "the server answered If-Range {} with the whole object",
                    sent.unwrap_or_default()
                ),
            });
        }
        if response.status != 206 {
            return Err(SourceError::RangeNotHonoured { detail: format!("status {}", response.status) });
        }
        if let Some((first, last, total)) = response.content_range {
            if first != range.start || last != range.end {
                return Err(SourceError::RangeNotHonoured {
                    detail: format!("asked for {}-{} and got {first}-{last}", range.start, range.end),
                });
            }
            if let Some(total) = total.filter(|&t| t != self.len) {
                return Err(SourceError::Changed {
                    detail: format!("opened at {} bytes, the object is now {total}", self.len),
                });
            }
        }
        if let Some(v) = validator.filter(|v| !v.starts_with("W/")) {
            let mut kept = lock()?;
            match kept.as_deref() {
                None => *kept = Some(v),
                Some(k) if k != v => {
                    return Err(SourceError::Changed {
                        detail: format!("opened as {k}, the range came as {v}"),
                    });
                }
                Some(_) => {}
            }
        }
        if response.body.len() as u64 != u64::from(len) {
            return Err(SourceError::Short { wanted: u64::from(len), got: response.body.len() as u64 });
        }
        Ok(Bytes::new(response.body))
    }
}

/// One request a [`Recording`] source saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// First byte asked for.
    pub offset: u64,
    /// Bytes asked for.
    pub len: u32,
}

/// Wraps a source and logs every range asked of it.
#[derive(Debug)]
pub struct Recording<S> {
    inner: S,
    log: Mutex<Vec<Request>>,
}

impl<S> Recording<S> {
    /// A recording source around `inner`.
    pub fn new(inner: S) -> Recording<S> {
        Recording { inner, log: Mutex::new(Vec::new()) }
    }

    /// The requests so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// The bytes asked for so far.
    pub fn bytes_requested(&self) -> u64 {
        self.requests().iter().map(|r| u64::from(r.len)).sum()
    }

    /// Whether any request touched `offset..offset + len`.
    pub fn touched(&self, offset: u64, len: u64) -> bool {
        self.requests().iter().any(|r| r.offset < offset + len && offset < r.offset + u64::from(r.len))
    }

    fn note(&self, offset: u64, len: u32) {
        if let Ok(mut l) = self.log.lock() {
            l.push(Request { offset, len });
        }
    }
}

impl<S: BlockSource> BlockSource for Recording<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        self.note(offset, len);
        self.inner.read_range(offset, len)
    }
}

impl<S: AsyncBlockSource> AsyncBlockSource for Recording<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    async fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        self.note(offset, len);
        self.inner.read_range(offset, len).await
    }
}
