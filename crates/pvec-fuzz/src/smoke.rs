//! A deterministic smoke driver: the seeds, mutated, through a target, for a number of executions.

use std::collections::BTreeMap;

/// Something a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// The target panicked.
    Panic {
        /// The input.
        input: Vec<u8>,
        /// The message.
        message: String,
    },
    /// An execution asked the allocator for more than the budget.
    Memory {
        /// The input.
        input: Vec<u8>,
        /// The bytes requested.
        requested: usize,
        /// The budget.
        budget: usize,
    },
}

/// What a run did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Executions.
    pub executions: u64,
    /// How often each word was reached.
    pub reached: BTreeMap<String, u64>,
    /// What went wrong.
    pub findings: Vec<Finding>,
}

/// Runs `target` for `executions` from the seeds of `seeds`, mutated from `rng`. `meter` runs a closure and
/// returns the bytes it requested from the allocator.
pub fn run(
    _target: &str,
    _seeds: &[Vec<u8>],
    _executions: u64,
    _rng: u64,
    _meter: &dyn Fn(&mut dyn FnMut()) -> usize,
) -> Report {
    todo!("P7.8")
}

/// [`run`] over any function of bytes, which is how a test gives the driver a target that misbehaves.
pub fn run_target(
    _target: impl Fn(&[u8]) -> String,
    _seeds: &[Vec<u8>],
    _executions: u64,
    _rng: u64,
    _meter: &dyn Fn(&mut dyn FnMut()) -> usize,
) -> Report {
    todo!("P7.8")
}
