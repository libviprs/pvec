//! Fuzzing the reader (issue P7.8): the six targets as plain functions over bytes, so the same code runs under
//! libFuzzer (`fuzz/`), under the deterministic smoke driver ([`smoke`]) and in an ordinary test.
//!
//! A target takes bytes, must not panic, hang or allocate in proportion to what the bytes claim, and returns
//! a short word for what it reached (`ok`, or the rule that refused), which the smoke driver counts so a run
//! can say how far past the first check its executions got.

mod layout;
pub mod mutate;
pub mod seeds;
pub mod smoke;
mod targets;

pub use targets::*;
