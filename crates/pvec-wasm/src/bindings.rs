//! The `wasm-bindgen` exports: the whole of [`crate::Runtime`] as numbers.
//!
//! Every export takes numbers or a slice of bytes or numbers and returns a number: a handle, a count, a
//! byte offset into linear memory, or a negative [`crate::AbiError::code`]. No export returns an object, an
//! array of objects or a string, so a call costs JS no allocation and a result costs it the four typed
//! arrays it makes itself over linear memory (`new Float64Array(memory.buffer, ptr, len)`). A test
//! reads this file and fails if an export says otherwise.
//!
//! The reader works on bytes in memory, so `open` takes the file whole. Range requests over HTTP are a
//! feature of the facade that is not there yet (`http` is a marker), and with it the range callback.

// SAFETY: the `#[wasm_bindgen]` expansion emits `unsafe` attributes and blocks of its own. Nothing in
// this file writes `unsafe` itself.
#![allow(unsafe_code)]

use crate::{AbiError, BufferKind, ElemType, Precision, Runtime, WASM_ABI_VERSION};
use pvec::{Bounds2, TessellationContext, Transform2};
use std::cell::RefCell;
use wasm_bindgen::prelude::wasm_bindgen;

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

fn code(r: Result<u32, AbiError>) -> i32 {
    match r {
        Ok(v) => v as i32,
        Err(e) => e.code(),
    }
}

fn unit(r: Result<(), AbiError>) -> i32 {
    code(r.map(|()| 0))
}

fn kind(which: u32) -> Result<BufferKind, AbiError> {
    [BufferKind::Vertices, BufferKind::Offsets, BufferKind::EntityIds, BufferKind::StyleRefs]
        .get(which as usize)
        .copied()
        .ok_or(AbiError::BadArgument)
}

/// The version of the packed layouts and of this export set.
#[wasm_bindgen]
pub fn wasm_abi_version() -> u32 {
    WASM_ABI_VERSION
}

/// Opens a file and returns its handle, or a negative code.
#[wasm_bindgen]
pub fn open(bytes: &[u8]) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().open(bytes.to_vec())))
}

/// The entities of `file` that meet the box, at most `budget` of them, as a batch handle or a negative code.
#[wasm_bindgen]
pub fn query_viewport(file: u32, min_x: f64, min_y: f64, max_x: f64, max_y: f64, budget: u32) -> i32 {
    let bounds = Bounds2 { min: [min_x, min_y], max: [max_x, max_y] };
    RUNTIME.with(|r| code(r.borrow_mut().query_viewport(file, bounds, budget)))
}

/// The number of entities in the batch, or a negative code.
#[wasm_bindgen]
pub fn batch_len(batch: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().batch_len(batch)))
}

/// 1 when the budget cut the batch short, 0 when not, or a negative code.
#[wasm_bindgen]
pub fn batch_truncated(batch: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().batch_truncated(batch).map(u32::from)))
}

/// Tessellates a batch. `chain` holds 15 numbers: the six of `source_to_map`, the six of `map_to_screen`
/// (each `[a, b, c, d, tx, ty]`, rows `[a b tx]` and `[c d ty]`), then `pixel_ratio`,
/// `max_screen_error_px` and `max_world_error`. `precision` is 0 for `Float64Array` vertices and 1 for
/// `Float32Array` vertices relative to the origin. Returns the handle of the packed result or a negative code.
#[wasm_bindgen]
pub fn tessellate_batch(batch: u32, chain: &[f64], max_segments: u32, precision: u32) -> i32 {
    let build = || -> Result<TessellationContext, AbiError> {
        if chain.len() != 15 {
            return Err(AbiError::BadArgument);
        }
        let map = |at: usize| {
            let m: [f64; 6] = chain[at..at + 6].try_into().map_err(|_| AbiError::BadArgument)?;
            Transform2::new(m).map_err(|_| AbiError::BadArgument)
        };
        Ok(TessellationContext {
            source_to_map: map(0)?,
            map_to_screen: map(6)?,
            pixel_ratio: chain[12],
            max_screen_error_px: chain[13],
            max_world_error: chain[14],
            max_segments: max_segments as usize,
        })
    };
    let precision = match precision {
        0 => Precision::F64,
        1 => Precision::F32,
        _ => return AbiError::BadArgument.code(),
    };
    match build() {
        Ok(ctx) => RUNTIME.with(|r| code(r.borrow_mut().tessellate_batch(batch, &ctx, precision))),
        Err(e) => e.code(),
    }
}

/// The number of elements of array `which` (0 vertices, 1 offsets, 2 entity ids, 3 style refs) of a
/// packed result, or a negative code. Read it before `buffer_ptr`.
#[wasm_bindgen]
pub fn buffer_len(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(kind(which).and_then(|k| r.borrow().view(result, k)).map(|v| v.len)))
}

/// The byte offset in linear memory of array `which` of a packed result, or 0 for a handle that is not one.
#[wasm_bindgen]
pub fn buffer_ptr(result: u32, which: u32) -> u32 {
    RUNTIME.with(|r| kind(which).and_then(|k| r.borrow().view(result, k)).map_or(0, |v| v.ptr))
}

/// The element type of array `which`: 0 `Float64Array`, 1 `Float32Array`, 2 `Uint32Array`, or a negative code.
#[wasm_bindgen]
pub fn buffer_elem(result: u32, which: u32) -> i32 {
    let elem = |v: crate::View| match v.elem {
        ElemType::F64 => 0,
        ElemType::F32 => 1,
        ElemType::U32 => 2,
    };
    RUNTIME.with(|r| code(kind(which).and_then(|k| r.borrow().view(result, k)).map(elem)))
}

/// The number of entities of the batch that got no vertices, or a negative code.
#[wasm_bindgen]
pub fn tessellation_failed(result: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().failed(result)))
}

/// The conformance tag of a packed result: 1, display fidelity, or a negative code.
#[wasm_bindgen]
pub fn tessellation_conformance(result: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().conformance(result)))
}

/// The x of the origin the `Float32Array` vertices are relative to; NaN for a handle that is not a result.
#[wasm_bindgen]
pub fn tessellation_origin_x(result: u32) -> f64 {
    RUNTIME.with(|r| r.borrow().origin(result).map_or(f64::NAN, |o| o[0]))
}

/// The y of the origin, as above.
#[wasm_bindgen]
pub fn tessellation_origin_y(result: u32) -> f64 {
    RUNTIME.with(|r| r.borrow().origin(result).map_or(f64::NAN, |o| o[1]))
}

/// The memory epoch: the size of linear memory in pages. Read it with the pointers; if it differs
/// later, every typed array made over `memory.buffer` before is detached.
#[wasm_bindgen]
pub fn memory_epoch() -> u32 {
    RUNTIME.with(|r| r.borrow().memory_epoch())
}

/// 0 when views read at `epoch` from `handle` are still usable, -2 when memory has grown since, -1 when the
/// handle is released.
#[wasm_bindgen]
pub fn view_current(handle: u32, epoch: u32) -> i32 {
    RUNTIME.with(|r| unit(r.borrow().view_current(handle, epoch)))
}

/// Grows linear memory by one page. It exists so a host can test its stale-view handling.
#[wasm_bindgen]
pub fn force_grow() {
    RUNTIME.with(|r| r.borrow_mut().force_grow());
}

/// Releases a handle of any kind: 0, or -1 when it is not live. A second release is -1.
#[wasm_bindgen]
pub fn release(handle: u32) -> i32 {
    RUNTIME.with(|r| unit(r.borrow_mut().release(handle)))
}
