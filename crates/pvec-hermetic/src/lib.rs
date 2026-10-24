//! Hermetic container lifecycle for the conformance suite (issue P2.1b).
//!
//! A corpus revision that needs a converter or oracle runs it in a container the suite starts for
//! the run and removes afterwards, on pass and on failure. This crate is that fixture, plus the
//! rule for what happens when the container cannot start. No image and no credential lives here.

mod container;
mod policy;

pub use container::{Config, Container, ContainerError, Exec};
pub use policy::{Decision, Policy, RunKind};
