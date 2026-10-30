//! The WASM-safe packed output interfaces (issue P7.3) over the `pvec` facade.
//!
//! A browser handed one JS object per entity spends its frame budget in the allocator, so a result
//! here is four typed arrays per batch and an integer handle, whatever the number of entities. The
//! layouts, the ownership rules and the compatibility rules are `docs/WASM_ABI.md`, which is the
//! contract `libviprs/maplibre-pvec` builds against.
//!
//! [`Runtime`] is the whole interface in plain Rust, so it runs and is tested natively. On
//! `wasm32` the `bindings` module exports it through `wasm-bindgen` as scalars only: handles, counts,
//! pointers and negative error codes. JS builds the typed arrays itself over linear memory.
#![deny(missing_docs)]

mod abi;
mod runtime;

#[cfg(target_arch = "wasm32")]
mod bindings;

pub use abi::{
    AbiError, BufferKind, DISPLAY_FIDELITY, ElemType, Precision, TIER0_BOUND_NOT_MET, TIER0_CHORD_POLYLINE,
    TIER0_CONTROL_POLYGON, TIER0_EVALUATOR_REFUSED, TIER0_EXACT, TIER0_INFEASIBLE_OTHER, TIER0_VERTEX_CAP, View,
    WASM_ABI_VERSION,
};
pub use runtime::{CellList, DirectoryInfo, Runtime, Tier0Stream};
