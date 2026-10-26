//! The stand-in for the `pvec-wasm` decode artefact of P8.2 (the real one is P7.3 and does not exist yet).
//!
//! What a browser needs to draw one viewport of a `.pvec`: open the container from bytes, read the spatial
//! directory, plan the viewport, decode the planned blocks (LZ4 and deflate included) and the advanced curve
//! set. That is exactly what `pvec_decode` does, behind the two exports a host needs. It is not the shipped
//! API, so the size it reports is a measurement of the decode path and a lower bound for the artefact.
#![allow(unsafe_code, missing_docs)]

use pvec_container::source::MemorySource;
use pvec_container::{ContainerReader, ReaderConfig};
use pvec_stream::{Aabb, SpatialDirectory, ViewportConfig, ViewportQuery};

/// Reserves `len` bytes in wasm memory for the host to write an artifact into.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

/// Opens the artifact at `ptr` (`len` bytes), plans the viewport and decodes what the plan holds. Returns the
/// number of hot entities decoded plus the advanced records, or `u64::MAX` when anything is refused.
#[unsafe(no_mangle)]
pub extern "C" fn pvec_decode(ptr: *const u8, len: usize, x0: f64, y0: f64, x1: f64, y1: f64) -> u64 {
    // SAFETY: the host wrote `len` bytes at the pointer `pvec_alloc` returned and keeps them alive for the call.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    decode(bytes, [x0, y0, x1, y1]).unwrap_or(u64::MAX)
}

fn decode(bytes: &[u8], w: [f64; 4]) -> Option<u64> {
    let r = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).ok()?;
    let dir = SpatialDirectory::open(&r).ok()??;
    let win = Aabb::new([w[0], w[1]], [w[2], w[3]]).ok()?;
    let plan = ViewportQuery::new(win, 1000.0 / (w[2] - w[0]).max(0.000_001))
        .plan(&dir, &r, &ViewportConfig { prefetch_margin_px: 0.0 })
        .ok()?;
    let blocks = r.read_blocks(&plan.block_ids()).ok()?;
    let mut n: u64 = blocks.iter().map(|b| b.hot().map_or(0, |h| h.len() as u64)).sum();
    if plan.advanced_in_view {
        for c in 0..r.advanced_chunk_count().ok()? {
            n += r.advanced(c).ok()??.view().ok()?.len() as u64;
        }
    }
    Some(n)
}
