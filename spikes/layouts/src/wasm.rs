//! Exports for the wasm32 build: each candidate's touch over a whole file already in linear memory.
//! The host allocates with `alloc`, copies the file in, and calls `touch_*`.
//!
//! This is the only module with `unsafe`. The host contract is documented on each function.
#![allow(unsafe_code)]

use crate::common::{Aligned, touch_file};

/// Allocates `n` bytes aligned to 16 and returns the address.
///
/// The allocation is intentionally leaked: the host owns its lifetime and the module is discarded
/// after a benchmark run.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut a = Box::new(Aligned::zeroed(n));
    let p = a.as_mut_bytes().as_mut_ptr();
    Box::leak(a);
    p
}

fn run<C: crate::common::Codec>(ptr: *const u8, len: usize) -> u64 {
    // SAFETY: the host passes an address returned by `alloc` together with the `n` it asked for, the
    // region has been initialised by the host's copy, and nothing else writes to it during the call.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    touch_file::<C>(bytes).map_or(u64::MAX, |(sum, n)| sum.wrapping_add(n))
}

/// Candidate B over `len` bytes at `ptr`. Returns the touch sum plus entity count, or `u64::MAX`.
#[unsafe(no_mangle)]
pub extern "C" fn touch_b(ptr: *const u8, len: usize) -> u64 {
    run::<crate::packed::Packed>(ptr, len)
}
/// Candidate A (iterator reads).
#[cfg(feature = "cand-a")]
#[unsafe(no_mangle)]
pub extern "C" fn touch_a(ptr: *const u8, len: usize) -> u64 {
    run::<crate::fbs_soa::FbSoa>(ptr, len)
}
/// Candidate A (raw byte view).
#[cfg(feature = "cand-a")]
#[unsafe(no_mangle)]
pub extern "C" fn touch_a_raw(ptr: *const u8, len: usize) -> u64 {
    run::<crate::fbs_soa::FbSoaRaw>(ptr, len)
}
/// Candidate C.
#[cfg(feature = "cand-c")]
#[unsafe(no_mangle)]
pub extern "C" fn touch_c(ptr: *const u8, len: usize) -> u64 {
    run::<crate::rk::Rkyv>(ptr, len)
}
/// Baseline.
#[cfg(feature = "cand-base")]
#[unsafe(no_mangle)]
pub extern "C" fn touch_base(ptr: *const u8, len: usize) -> u64 {
    run::<crate::fbs_obj::FbObj>(ptr, len)
}
