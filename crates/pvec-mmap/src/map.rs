//! The one `unsafe` block of the mapped source, and nothing else (ADR 0016).
//!
//! The workspace allows `unsafe` in exactly two files: the casts in `pvec-container`'s
//! `blocks/aligned.rs`, and this one. `crates/pvec-checks/tests/issue_108_unsafe_policy.rs` counts the
//! tokens (one here), requires a `SAFETY:` comment within five lines above each, and fails if either
//! number moves.
//!
//! Why the block is sound enough to ship: `Mmap::map` is unsafe because the file can change under the
//! map. Another process that writes to it changes the bytes a slice of the map shows (a data race by the
//! language's rules), and one that truncates it makes a touch of the missing pages raise SIGBUS, which
//! cannot be caught. This crate cannot rule that out, so it limits what a change can reach. The file is
//! opened read-only, the map is never handed out (`read_range` returns a copy, never a borrow), and every
//! byte copied is untrusted input that the reader validates, so a changed byte is a wrong value and not a
//! broken invariant. A file that has shrunk since it was mapped is detected before each copy; the window
//! between that check and the copy is the residual risk, and the crate documentation says to map only
//! files the process owns or a read-only copy.
#![allow(unsafe_code)]

use memmap2::Mmap;
use std::fs::File;
use std::io;

/// Maps `file` read-only, for its whole length.
pub(crate) fn map_read_only(file: &File) -> io::Result<Mmap> {
    // SAFETY: the file is read-only and the map never escapes this crate as a borrow; the bytes are
    // copied out and validated. A concurrent truncation can still fault (the residual risk in the
    // module documentation, narrowed by the length check before each copy).
    unsafe { Mmap::map(file) }
}
