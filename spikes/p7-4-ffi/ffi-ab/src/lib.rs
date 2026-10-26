//! Candidates (a) and (b) of the P7.4 ADR: a hand-written `extern "C"` facade over opaque handles, with
//! `#[repr(C)]` plain data only. The two differ in the header, not in this code: (a) ships
//! `include/pvec_a.h` as written by hand, (b) ships `include/pvec_b.h` as cbindgen generates it.
#![allow(unsafe_code)]
#![deny(improper_ctypes_definitions)]

use pvec::PvecReader;

/// A file that is open. C sees a pointer to this and nothing of what is behind it.
#[repr(C)]
pub struct PvecHandle {
    _opaque: [u8; 0],
}

/// What a call returns. Zero is success.
#[repr(i32)]
pub enum PvecStatus {
    /// Done.
    Ok = 0,
    /// A null pointer or an out-of-range argument.
    BadArgument = -1,
    /// The bytes are not a PVec file the reader accepts.
    Read = -2,
}

/// Opens the file in `bytes[..len]` and stores a handle in `*out`.
///
/// # Safety
///
/// `bytes` must point at `len` readable bytes and `out` at a writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pvec_open(bytes: *const u8, len: usize, out: *mut *mut PvecHandle) -> PvecStatus {
    if bytes.is_null() || out.is_null() {
        return PvecStatus::BadArgument;
    }
    // SAFETY: the caller promises `len` readable bytes at `bytes`.
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    match PvecReader::open(slice.to_vec()) {
        Ok(reader) => {
            // SAFETY: the caller promises `out` is writable.
            unsafe { *out = Box::into_raw(Box::new(reader)).cast::<PvecHandle>() };
            PvecStatus::Ok
        }
        Err(_) => PvecStatus::Read,
    }
}

/// Stores the number of entities of the open file in `*out`.
///
/// # Safety
///
/// `handle` must come from `pvec_open` and not be closed, and `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pvec_entity_count(handle: *const PvecHandle, out: *mut u64) -> PvecStatus {
    if handle.is_null() || out.is_null() {
        return PvecStatus::BadArgument;
    }
    // SAFETY: `handle` came from `pvec_open`, which made it from a `Box<PvecReader>`.
    let reader = unsafe { &*handle.cast::<PvecReader>() };
    match reader.entity_count() {
        Ok(n) => {
            // SAFETY: the caller promises `out` is writable.
            unsafe { *out = n };
            PvecStatus::Ok
        }
        Err(_) => PvecStatus::Read,
    }
}

/// Closes a handle. Null is allowed and does nothing.
///
/// # Safety
///
/// `handle` must come from `pvec_open` and not have been closed already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pvec_close(handle: *mut PvecHandle) {
    if !handle.is_null() {
        // SAFETY: `handle` came from `pvec_open`, which made it from a `Box<PvecReader>`.
        drop(unsafe { Box::from_raw(handle.cast::<PvecReader>()) });
    }
}
