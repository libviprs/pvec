//! The conformance harness (EPIC P, phase 2).
//!
//! A green tick means different things depending on where a fixture's truth came from. This crate
//! enforces EPIC O's four evidence classes in the harness and does not redefine them: the class of a
//! fixture is read from EPIC O's manifest and never derived, upgraded or extended here.

mod error;
mod evidence;

pub use error::{ConformanceError, Result};
pub use evidence::{Assertion, Outcome, Registry, Tally, Verdict, check_report, may_assert, render, tally};
