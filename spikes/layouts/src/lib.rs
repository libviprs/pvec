//! P1.5 spike: physical layout candidates on the shared fixtures. Throwaway; the recommendation is in
//! `docs/spikes/01-5-layouts.md`.
#![allow(clippy::needless_range_loop)]
// The hot loops stay as measured (chunks_exact, % 8) instead of the newer std spellings clippy prefers.
#![allow(clippy::chunks_exact_to_as_chunks, clippy::manual_is_multiple_of)]

pub mod common;
#[cfg(feature = "cand-base")]
pub mod fbs_obj;
#[cfg(feature = "cand-a")]
pub mod fbs_soa;
pub mod packed;
#[cfg(feature = "cand-c")]
pub mod rk;
pub mod wasm;

#[cfg(feature = "cand-a")]
#[rustfmt::skip]
#[allow(warnings, clippy::all)]
#[path = "gen/pvec_soa_generated.rs"]
pub(crate) mod soa_gen;

#[cfg(feature = "cand-base")]
#[rustfmt::skip]
#[allow(warnings, clippy::all)]
#[path = "gen/pvec_obj_generated.rs"]
pub(crate) mod obj_gen;
