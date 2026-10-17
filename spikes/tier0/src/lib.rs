//! P1.7 spike: Tier 0 = exact cheap geometry + bounded-error previews for deferred entities.
//!
//! Throwaway. Every preview here is display fidelity only: a bounded rendering approximation under
//! a stated screen-space error budget. None of it is geometrically or mathematically exact, and the
//! exact entity it stands in for is always the one that supersedes it.

pub mod model;
pub mod preview;
pub mod tier;

pub use pvec_fixtures::Pt;
