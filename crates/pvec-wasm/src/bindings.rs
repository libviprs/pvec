//! The `wasm-bindgen` exports: the whole of [`crate::Runtime`] as numbers.
//!
//! Every export takes numbers or a slice of bytes or numbers and returns a number: a handle, a count, a
//! byte offset into linear memory, or a negative [`crate::AbiError::code`]. No export returns an object, an
//! array of objects or a string, so a call costs JS no allocation and a result costs it the four typed
//! arrays it makes itself over linear memory (`new Float64Array(memory.buffer, ptr, len)`). A test
//! reads this file and fails if an export says otherwise.
//!
//! Version 2 adds the spatial directory and the Tier 0 stream (`directory_*`, `cells_*`, `tier0_*`), all
//! of the same shape: a handle, then lengths, offsets and element types of the arrays behind it.
//!
//! `open` takes the file whole. `open_ranged`, `provide_range`, `open_poll` and `needed_*` (issue #205) open
//! it by byte ranges the host supplies, and a call that needs one that is missing returns -7.

// SAFETY: the `#[wasm_bindgen]` expansion emits `unsafe` attributes and blocks of its own. Nothing in
// this file writes `unsafe` itself.
#![allow(unsafe_code)]

use crate::{AbiError, BufferKind, ElemType, Precision, Runtime, View, WASM_ABI_VERSION};
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
        ElemType::U8 => 3,
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

fn elem(v: View) -> u32 {
    match v.elem {
        ElemType::F64 => 0,
        ElemType::F32 => 1,
        ElemType::U32 => 2,
        ElemType::U8 => 3,
    }
}

fn window(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Bounds2 {
    Bounds2 { min: [min_x, min_y], max: [max_x, max_y] }
}

/// One number about the spatial directory of a file, or a negative code (-4 when the reader refused the
/// directory): 0 whether the file has one (0 or 1; the rest are 0 when it has not), 1 leaves, 2 height,
/// 3 hot entities in the leaves, 4 leaves with a Tier 0 block, 5 whether the advanced curves have bounds.
#[wasm_bindgen]
pub fn directory_stat(file: u32, which: u32) -> i32 {
    RUNTIME.with(|r| {
        code(r.borrow().directory(file).and_then(|d| match (which, d) {
            (0, d) => Ok(u32::from(d.is_some())),
            (1..=5, None) => Ok(0),
            (1, Some(d)) => Ok(d.cells),
            (2, Some(d)) => Ok(d.height),
            (3, Some(d)) => Ok(d.entities),
            (4, Some(d)) => Ok(d.tier0_leaves),
            (5, Some(d)) => Ok(u32::from(d.advanced_bounds.is_some())),
            _ => Err(AbiError::BadArgument),
        }))
    })
}

/// A corner of a box of the directory: 0 to 3 are `min_x, min_y, max_x, max_y` of the whole tree, 4 to 7
/// the same for the advanced curves. NaN when the file has no directory, the box does not exist, or the
/// handle is not a file.
#[wasm_bindgen]
pub fn directory_bound(file: u32, which: u32) -> f64 {
    RUNTIME.with(|r| {
        let Ok(Some(d)) = r.borrow().directory(file) else { return f64::NAN };
        let b = if which < 4 { Some(d.bounds) } else { d.advanced_bounds };
        b.and_then(|b| [b.min[0], b.min[1], b.max[0], b.max[1]].get(which as usize % 4).copied())
            .unwrap_or(f64::NAN)
    })
}

/// The leaves of the directory that touch the box, in ascending cell id, as a result handle of five
/// arrays, or a negative code (-6 when the file has no directory).
#[wasm_bindgen]
pub fn directory_query(file: u32, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().directory_query(file, window(min_x, min_y, max_x, max_y))))
}

/// The number of elements of array `which` of a `directory_query` result (0 cell ids, 1 entities, 2 has
/// Tier 0, 3 stored bytes, 4 bounds, four per leaf), or a negative code.
#[wasm_bindgen]
pub fn cells_len(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().cells_view(result, which).map(|v| v.len)))
}

/// The byte offset in linear memory of that array, or 0 for a handle or number that is not one.
#[wasm_bindgen]
pub fn cells_ptr(result: u32, which: u32) -> u32 {
    RUNTIME.with(|r| r.borrow().cells_view(result, which).map_or(0, |v| v.ptr))
}

/// The element type of that array: 0 `Float64Array`, 2 `Uint32Array`, or a negative code.
#[wasm_bindgen]
pub fn cells_elem(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().cells_view(result, which).map(elem)))
}

/// The Tier 0 stream of the box: a result handle of five arrays, or a negative code (-6 when the file
/// has no directory, -4 when the directory or a Tier 0 block is damaged).
#[wasm_bindgen]
pub fn tier0_viewport(file: u32, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().tier0_viewport(file, window(min_x, min_y, max_x, max_y))))
}

/// One number about a Tier 0 result, or a negative code: 0 entries, 1 exact, 2 previews, 3 infeasible,
/// 4 entities of the box no entry covers, 5 leaves read, 6 leaves with no Tier 0 block, 7 whether the
/// advanced curves meet the box.
#[wasm_bindgen]
pub fn tier0_stat(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().tier0_stat(result, which)))
}

/// The number of elements of array `which` of a Tier 0 result (0 entity ids, 1 codes, 2 declared errors,
/// 3 offsets, 4 vertices), or a negative code.
#[wasm_bindgen]
pub fn tier0_buffer_len(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().tier0_view(result, which).map(|v| v.len)))
}

/// The byte offset in linear memory of that array, or 0 for a handle or number that is not one.
#[wasm_bindgen]
pub fn tier0_buffer_ptr(result: u32, which: u32) -> u32 {
    RUNTIME.with(|r| r.borrow().tier0_view(result, which).map_or(0, |v| v.ptr))
}

/// The element type of that array: 0 `Float64Array`, 2 `Uint32Array`, or a negative code.
#[wasm_bindgen]
pub fn tier0_buffer_elem(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().tier0_view(result, which).map(elem)))
}

/// One number about the rasters of `file`, or a negative code: 0 rasters, 1 rasters skipped, 2 whether the
/// file has an image this reader cannot show (`Effect::Unrastered`), 3 whether it has a raster stream. A
/// module that exports this one has the raster calls; the ABI version is still 2.
#[wasm_bindgen]
pub fn raster_stat(file: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().raster_stat(file, which)))
}

/// For the `i`th skipped raster (ascending raster id): `which` 0 its `RasterId`, 1 the number of the `RASTER-NN`
/// rule that skipped it. A negative code for a number that is not one.
#[wasm_bindgen]
pub fn raster_skipped(file: u32, i: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().raster_skipped(file, i, which)))
}

/// The placement of raster `raster` as a result handle of four arrays (`raster_buffer_*`), or a negative code.
#[wasm_bindgen]
pub fn raster_placement(file: u32, raster: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().raster_placement(file, raster)))
}

/// The tiles every raster needs for the box at `scale` display pixels per source unit: a result handle of nine
/// arrays (`raster_buffer_*`), or a negative code (-7 until the index blocks it needs are provided).
/// `max_decoded` is `LIMIT-16` in bytes, 0 for the module's.
#[wasm_bindgen]
pub fn raster_plan(
    file: u32,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    scale: f64,
    max_decoded: f64,
) -> i32 {
    RUNTIME.with(|r| {
        code(r.borrow_mut().raster_plan(file, window(min_x, min_y, max_x, max_y), scale, max_decoded))
    })
}

/// The encoded bytes of one tile as a result handle whose array 0 is a `Uint8Array`, or a negative code: -7
/// until the tile's range is provided, -8 no raster stream, -9 not stored, -10 raster skipped, -3 a bad
/// raster, level, column or row, -4 a failed checksum.
#[wasm_bindgen]
pub fn raster_tile(file: u32, raster: u32, level: u32, col: u32, row: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().raster_tile(file, raster, level, col, row)))
}

/// The number of elements of array `which` of a raster result, or a negative code.
#[wasm_bindgen]
pub fn raster_buffer_len(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().raster_view(result, which).map(|v| v.len)))
}

/// The byte offset in linear memory of that array, or 0 for a handle or number that is not one.
#[wasm_bindgen]
pub fn raster_buffer_ptr(result: u32, which: u32) -> u32 {
    RUNTIME.with(|r| r.borrow().raster_view(result, which).map_or(0, |v| v.ptr))
}

/// The element type of that array: 0 `Float64Array`, 2 `Uint32Array`, 3 `Uint8Array`, or a negative code.
#[wasm_bindgen]
pub fn raster_buffer_elem(result: u32, which: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().raster_view(result, which).map(elem)))
}

/// Starts opening a file of `len` bytes that the host serves by byte ranges, and returns a handle, or a
/// negative code (-3 for a length that is not a whole number from 1 up). Nothing is read yet: call
/// `open_poll` (still ABI version 2: a module that has this export has the range calls).
#[wasm_bindgen]
pub fn open_ranged(len: f64) -> i32 {
    match whole(len) {
        Some(len) => RUNTIME.with(|r| code(r.borrow_mut().open_ranged(len))),
        None => AbiError::BadArgument.code(),
    }
}

/// Hands the module the bytes at `offset` of a range-opened file (or of the handle `open_ranged` returned):
/// 0, or a negative code (-3 when the range is not inside the file). It may be more than was asked for.
#[wasm_bindgen]
pub fn provide_range(handle: u32, offset: f64, bytes: &[u8]) -> i32 {
    match whole(offset) {
        Some(offset) => RUNTIME.with(|r| unit(r.borrow_mut().provide_range(handle, offset, bytes))),
        None => AbiError::BadArgument.code(),
    }
}

/// Tries to finish opening: the handle `open_ranged` returned, which is then the file's, -7 when ranges are
/// missing (see `needed_count`), -4 when what was provided is not a PVec file.
#[wasm_bindgen]
pub fn open_poll(handle: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow_mut().open_poll(handle)))
}

/// How many ranges the last call on `handle` that returned -7 still needs, or a negative code.
#[wasm_bindgen]
pub fn needed_count(handle: u32) -> i32 {
    RUNTIME.with(|r| code(r.borrow().needed(handle).map(|n| n.len() as u32)))
}

/// The first byte of range `i` of the needed list, or NaN when there is none.
#[wasm_bindgen]
pub fn needed_offset(handle: u32, i: u32) -> f64 {
    RUNTIME.with(|r| {
        r.borrow()
            .needed(handle)
            .ok()
            .and_then(|n| n.get(i as usize).map(|(o, _)| *o as f64))
            .unwrap_or(f64::NAN)
    })
}

/// The length of range `i` of the needed list, or a negative code.
#[wasm_bindgen]
pub fn needed_len(handle: u32, i: u32) -> i32 {
    RUNTIME.with(|r| {
        code(
            r.borrow()
                .needed(handle)
                .and_then(|n| n.get(i as usize).map(|(_, l)| *l).ok_or(AbiError::BadArgument)),
        )
    })
}

/// `v` as a whole number of bytes (an `f64` holds one exactly up to 2^53), or `None`.
fn whole(v: f64) -> Option<u64> {
    (v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= 9_007_199_254_740_992.0).then_some(v as u64)
}
