//! A browser-side HDF5 reader candidate: the pure-Rust `hdf5-reader` compiled to wasm32.
//!
//! Exports a bump allocator and two decoders over an HDF5 file image in linear memory. The decoders
//! walk the same datasets as the native readers and fold every value into an FNV-1a hash, so the
//! throughput measured is the cost of getting the numbers out of the file and nothing downstream.
#![allow(unsafe_code)]

use hdf5_reader::Hdf5File;

fn fnv_f64(h: &mut u64, x: f64) {
    for b in x.to_bits().to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
}

fn fnv_u32(h: &mut u64, x: u32) {
    for b in x.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
}

/// Allocates `n` bytes that the host fills with the file image, and leaks them for the call.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

fn image<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    // SAFETY: the host obtained `ptr` from `alloc(len)` and wrote exactly `len` initialised bytes
    // into it before calling, and nothing frees or mutates it during the call.
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

/// Decodes the H2 layout (type-batched arrays): every fixed-width coordinate dataset and the
/// polyline offsets. Returns the hash, or 0 if the file did not open.
#[unsafe(no_mangle)]
pub extern "C" fn h2_decode(ptr: *const u8, len: usize) -> u64 {
    let Ok(f) = Hdf5File::from_bytes(image(ptr, len)) else { return 0 };
    let mut h = 0xcbf29ce484222325u64;
    for name in
        ["line", "circle", "arc", "ellipse", "poly_pts", "sp_cps", "sp_knots", "sp_w", "comp_vals", "bbox"]
    {
        if let Ok(ds) = f.dataset(&format!("pvec_h2/{name}")) {
            if let Ok(a) = ds.read_array::<f64>() {
                for x in a.iter() {
                    fnv_f64(&mut h, *x);
                }
            }
        }
    }
    for name in ["poly_off", "sp_cp_off", "sp_knot_off", "sp_w_off", "comp_off"] {
        if let Ok(ds) = f.dataset(&format!("pvec_h2/{name}")) {
            if let Ok(a) = ds.read_array::<u32>() {
                for x in a.iter() {
                    fnv_u32(&mut h, *x);
                }
            }
        }
    }
    h
}

/// Decodes the point and direction tables of the H1 layout (compound types) and resolves every
/// polyline's variable-length list of point references through the global heap. Files must have
/// been written with the newest format (see the spike document: this reader mis-parses compound
/// members in the oldest format).
#[unsafe(no_mangle)]
pub extern "C" fn h1_decode(ptr: *const u8, len: usize) -> u64 {
    let Ok(f) = Hdf5File::from_bytes(image(ptr, len)) else { return 0 };
    let mut h = 0xcbf29ce484222325u64;
    for name in ["CARTESIAN_POINT", "DIRECTION"] {
        if let Ok(ds) = f.dataset(&format!("data/{name}")) {
            if let Ok(b) = ds.read_native_bytes() {
                for c in b.chunks_exact(8) {
                    fnv_f64(&mut h, f64::from_le_bytes(c.try_into().unwrap()));
                }
            }
        }
    }
    if let Ok(ds) = f.dataset("data/POLYLINE") {
        if let Ok(rows) = ds.read_native_bytes() {
            let r = ds.vlen_reference_size();
            for row in rows.chunks_exact(r) {
                if let Ok(bytes) = ds.resolve_vlen_reference_bytes(row, 4) {
                    for c in bytes.chunks_exact(4) {
                        fnv_u32(&mut h, u32::from_le_bytes(c.try_into().unwrap()));
                    }
                }
            }
        }
    }
    h
}
