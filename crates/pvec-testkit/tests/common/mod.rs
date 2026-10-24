//! Shared helpers: the vendored bundle, and a builder for records the corpus does not have, so a
//! test can hand the generator exactly one defect.
#![allow(dead_code)]

use pvec_corpus::Bundle;

pub fn bundle() -> Bundle {
    Bundle::vendored().expect("the vendored bundle opens")
}

pub const REVISION: &str = "r0-open";

#[allow(unused_imports)]
pub use pvec_testkit::records::*;

/// The record's entities as a rotated copy, for order tests: a fixed shuffle.
pub fn permutations(n: usize) -> Vec<Vec<usize>> {
    let id: Vec<usize> = (0..n).collect();
    let rev: Vec<usize> = (0..n).rev().collect();
    let rot: Vec<usize> = (0..n).map(|i| (i + n / 2 + 1) % n).collect();
    let evens_odds: Vec<usize> = (0..n).step_by(2).chain((1..n).step_by(2)).collect();
    vec![id, rev, rot, evens_odds]
}
