//! A deterministic smoke driver: the seeds, mutated, through a target, for a number of executions.
//!
//! This is not libFuzzer and does not pretend to be coverage guided. It is the same targets run from the same
//! seeds with the same structure-aware mutator, on stable Rust, with the allocator metered, so a pull request
//! can be gated without a nightly toolchain and a run can quote its counts. The coverage-guided runs are
//! `tools/fuzz.sh` over `fuzz/`.

use crate::TARGETS;
use crate::mutate::mutate;
use pvec_container::Limits;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// An execution that takes longer than this is a finding: it is the hang that libFuzzer's timeout would catch.
pub const SLOW: Duration = Duration::from_secs(5);

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
    /// An execution took longer than [`SLOW`].
    Slow {
        /// The input.
        input: Vec<u8>,
        /// How long it took.
        millis: u128,
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

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Byte-level havoc: flips, edge values, a chunk inserted, a chunk removed, a chunk repeated.
fn havoc(data: &mut Vec<u8>, rng: &mut Rng) {
    for _ in 0..1 + rng.below(4) {
        if data.is_empty() {
            data.push(rng.next() as u8);
            continue;
        }
        let at = rng.below(data.len());
        match rng.below(6) {
            0 => data[at] ^= 1 << rng.below(8),
            1 => data[at] = [0, 1, 0x7F, 0x80, 0xFF][rng.below(5)],
            2 => {
                let n = 1 + rng.below(8);
                data.splice(at..at, (0..n).map(|_| rng.next() as u8));
            }
            3 => {
                let n = (1 + rng.below(16)).min(data.len() - at);
                data.drain(at..at + n);
            }
            4 => {
                let n = (1 + rng.below(32)).min(data.len() - at);
                let chunk: Vec<u8> = data[at..at + n].to_vec();
                data.splice(at..at, chunk);
            }
            _ => {
                let w = 4.min(data.len() - at);
                data[at..at + w].copy_from_slice(&u32::MAX.to_le_bytes()[..w]);
            }
        }
    }
    data.truncate(1 << 20);
}

/// Runs `target` for `executions` from the seeds of `seeds`, mutated from `rng`. `meter` runs a closure and
/// returns the bytes it requested from the allocator.
pub fn run(
    target: &str,
    seeds: &[Vec<u8>],
    executions: u64,
    rng: u64,
    meter: &dyn Fn(&mut dyn FnMut()) -> usize,
) -> Report {
    let f =
        TARGETS.iter().find(|(n, _)| *n == target).unwrap_or_else(|| panic!("no target named {target}")).1;
    run_target(f, seeds, executions, rng, meter)
}

/// [`run`] over any function of bytes, which is how a test gives the driver a target that misbehaves.
pub fn run_target(
    target: impl Fn(&[u8]) -> String,
    seeds: &[Vec<u8>],
    executions: u64,
    rng: u64,
    meter: &dyn Fn(&mut dyn FnMut()) -> usize,
) -> Report {
    let mut rng = Rng(rng.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let budget = Limits::DEFAULT.max_alloc_bytes as usize;
    let mut report = Report::default();
    for n in 0..executions {
        let mut input = if seeds.is_empty() { Vec::new() } else { seeds[rng.below(seeds.len())].clone() };
        // the first pass over the seeds runs them as they are
        if n >= seeds.len() as u64 {
            match rng.below(3) {
                0 => havoc(&mut input, &mut rng),
                1 => mutate(&mut input, 1 << 20, rng.next()),
                _ => {
                    mutate(&mut input, 1 << 20, rng.next());
                    havoc(&mut input, &mut rng);
                }
            }
        } else if !seeds.is_empty() {
            input = seeds[n as usize].clone();
        }
        let mut word = String::new();
        let started = Instant::now();
        let mut requested = 0;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            requested = meter(&mut || word = target(&input));
        }));
        report.executions += 1;
        match outcome {
            Err(e) => {
                let message = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "a panic".to_string());
                *report.reached.entry("panic".into()).or_insert(0) += 1;
                report.findings.push(Finding::Panic { input, message });
            }
            Ok(()) => {
                *report.reached.entry(word).or_insert(0) += 1;
                if requested > budget {
                    report.findings.push(Finding::Memory { input: input.clone(), requested, budget });
                }
                if started.elapsed() > SLOW {
                    report.findings.push(Finding::Slow { input, millis: started.elapsed().as_millis() });
                }
            }
        }
    }
    report
}
