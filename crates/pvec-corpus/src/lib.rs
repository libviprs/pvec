//! Loader for the known-truth fixture bundle (issue P2.1).
//!
//! The bundle is the `r0-open` revision of `libviprs/pvec-gold-fixtures`, vendored as one
//! deterministic tar under `tests/corpus/` and pinned by its SHA-256 and revision. See
//! `docs/CORPUS.md` for what it proves and what it does not.
//!
//! Nothing in this crate opens a socket, starts a process or reads outside the repository.

pub mod bundle;
pub mod compare;
pub mod error;
pub mod expected;
pub mod json;
pub mod pin;
pub mod registry;
pub mod scalar;
pub mod sha256;
pub mod tar;

pub use bundle::Bundle;
pub use error::{CorpusError, Result};
pub use expected::ExpectedMath;
pub use pin::Pin;
pub use registry::{Claim, Fixture, FixtureRegistry};
pub use scalar::Scalar;
