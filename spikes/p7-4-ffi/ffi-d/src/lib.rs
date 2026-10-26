//! Candidate (d) of the P7.4 ADR: the P7.3 packed-buffer API, callable from C. Handles are `u32`, results
//! are four arrays, and a pointer that leaves the library points at a number array. No Rust-layout type
//! has a place in a signature, because there is no struct in it at all.
#![allow(unsafe_code)]
#![deny(improper_ctypes_definitions)]

use pvec::{Bounds2, TessellationContext, Transform2};
use pvec_wasm::{AbiError, BufferKind, Precision, Runtime, WASM_ABI_VERSION};
use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

fn code(r: Result<u32, AbiError>) -> i32 {
    match r {
        Ok(v) => v as i32,
        Err(e) => e.code(),
    }
}

/// The version of the packed layouts, the same number as `wasm_abi_version`.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_d_abi_version() -> u32 {
    WASM_ABI_VERSION
}

/// Opens the file in `bytes[..len]`; a handle, or a negative code.
///
/// # Safety
///
/// `bytes` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pvec_d_open(bytes: *const u8, len: usize) -> i32 {
    if bytes.is_null() {
        return AbiError::BadArgument.code();
    }
    // SAFETY: the caller promises `len` readable bytes at `bytes`.
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    RUNTIME.with(|r| code(r.borrow_mut().open(slice.to_vec())))
}

/// The entities that meet the box, at most `budget`; a batch handle, or a negative code.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_d_query_viewport(
    file: u32,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    budget: u32,
) -> i32 {
    let bounds = Bounds2 { min: [min_x, min_y], max: [max_x, max_y] };
    RUNTIME.with(|r| code(r.borrow_mut().query_viewport(file, bounds, budget)))
}

/// The number of entities in a batch, or a negative code.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_d_batch_len(batch: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().batch_len(batch)))
}

/// Tessellates a batch; `chain` holds the 15 numbers of `docs/WASM_ABI.md`. A result handle, or a negative code.
///
/// # Safety
///
/// `chain` must point at `chain_len` readable numbers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pvec_d_tessellate_batch(
    batch: u32,
    chain: *const f64,
    chain_len: usize,
    max_segments: u32,
    precision: u32,
) -> i32 {
    if chain.is_null() || chain_len != 15 {
        return AbiError::BadArgument.code();
    }
    // SAFETY: the caller promises `chain_len` readable numbers at `chain`.
    let c = unsafe { std::slice::from_raw_parts(chain, chain_len) };
    let (Ok(a), Ok(b)) = (
        Transform2::new([c[0], c[1], c[2], c[3], c[4], c[5]]),
        Transform2::new([c[6], c[7], c[8], c[9], c[10], c[11]]),
    ) else {
        return AbiError::BadArgument.code();
    };
    let ctx = TessellationContext {
        source_to_map: a,
        map_to_screen: b,
        pixel_ratio: c[12],
        max_screen_error_px: c[13],
        max_world_error: c[14],
        max_segments: max_segments as usize,
    };
    let precision = if precision == 0 { Precision::F64 } else { Precision::F32 };
    RUNTIME.with(|r| code(r.borrow_mut().tessellate_batch(batch, &ctx, precision)))
}

/// The number of elements of array `which` (0 vertices, 1 offsets, 2 entity ids, 3 style refs), or a negative code.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_d_buffer_len(result: u32, which: u32) -> i32 {
    let kind = [BufferKind::Vertices, BufferKind::Offsets, BufferKind::EntityIds, BufferKind::StyleRefs];
    let Some(kind) = kind.get(which as usize) else { return AbiError::BadArgument.code() };
    RUNTIME.with(|r| code(r.borrow().view(result, *kind).map(|v| v.len)))
}

/// The address of array `which`, valid until the result is released or the library is next entered
/// to release or tessellate; null for a handle that is not a result.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_d_buffer_ptr(result: u32, which: u32) -> *const c_void {
    RUNTIME.with(|r| {
        let r = r.borrow();
        match which {
            0 => r.vertices_f64(result).map(|s| s.as_ptr().cast()).unwrap_or(std::ptr::null()),
            1 => r.offsets(result).map(|s| s.as_ptr().cast()).unwrap_or(std::ptr::null()),
            2 => r.entity_ids(result).map(|s| s.as_ptr().cast()).unwrap_or(std::ptr::null()),
            3 => r.style_refs(result).map(|s| s.as_ptr().cast()).unwrap_or(std::ptr::null()),
            _ => std::ptr::null(),
        }
    })
}

/// Releases a handle of any kind: 0, or -1 when it is not live.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_d_release(handle: u32) -> i32 {
    RUNTIME.with(|r| match r.borrow_mut().release(handle) {
        Ok(()) => 0,
        Err(e) => e.code(),
    })
}
