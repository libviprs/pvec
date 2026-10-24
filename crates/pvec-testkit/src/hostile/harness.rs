//! The harness: holds any reader to every case it claims to provide.
//!
//! A reader is anything that takes bytes and answers [`Verdict`]. The reference reader is one; P4.5's
//! will be another, with a small adapter. For each case the harness reads the mutated bytes on a
//! thread of its own and checks, in this order:
//!
//! 1. **no panic** (an unwrap, an index, an overflow in a debug build, an `abort` through `catch_unwind`
//!    is a failure of the reader, never of the test run);
//! 2. **no hang**: a case that does not finish inside [`Limits::case_timeout`] is a failure and the run
//!    carries on with the next one (the stuck thread is abandoned);
//! 3. **a typed refusal naming the rule**: an accepted hostile file is a failure, and so is a refusal
//!    whose rule is not one the case allows;
//! 4. **a bounded allocation, measured before it can matter**: the bytes requested while reading must
//!    stay within [`Limits::alloc_per_byte`] times the input plus [`Limits::alloc_floor`]. The
//!    measurement is a counting allocator ([`Meter`]), not a clock, so a file that declares 2^32
//!    elements is refused for the allocation it would have made even on a fast machine;
//! 5. **the repaired file is accepted**: the base the mutation was made from reads fine, which is what
//!    shows the case failed because of its own mutation and not an unrelated defect.
//!
//! The counting allocator has to be a `#[global_allocator]` and so needs `unsafe`; this crate has
//! none, so the allocator lives in the test binary (`tests/support/counting_alloc.rs`) and is handed in
//! through [`Meter`]. It counts the thread that calls it: a reader that hands the work to another
//! thread is not measured, so such a reader has to be given a meter that counts the process (and
//! the tests that use it must not share the process with other work).

use super::bases::Base;
use super::catalog::{Case, Expect, Requires};
use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A refusal, as the harness sees it: the rule that was violated and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The violated rule: `DIR-nn`, `HOT-nn`, `ARN-nn`, `STREAM-nn`, a `ContainerError` variant name,
    /// `LAYOUT-nn`, `CONSIST-nn`, `LIMIT-nn`, `INTEGRITY-nn`.
    pub rule: String,
    /// What was wrong.
    pub detail: String,
}

/// What a reader says about a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It read the file as a complete, valid artifact.
    Accepted,
    /// It refused, naming the rule.
    Refused(Refusal),
}

/// A reader the corpus can be run against.
pub trait Reader: Send + Sync {
    /// A name for reports.
    fn name(&self) -> String;
    /// What it provides beyond the parsers. [`Requires::Parse`] is always assumed.
    fn provides(&self) -> Vec<Requires>;
    /// Reads a file. It must not panic, hang or allocate in proportion to what the file claims.
    fn read(&self, bytes: &[u8]) -> Verdict;
}

/// Counts the bytes a closure requests from the allocator.
pub trait Meter: Send + Sync {
    /// Runs `f` and returns the bytes it requested on this thread.
    fn measure(&self, f: &mut dyn FnMut()) -> usize;
}

/// A meter that measures nothing, for a test binary without the counting allocator. Every allocation
/// check then passes trivially, so a test that is about allocation must not use it.
#[derive(Clone, Copy, Debug)]
pub struct NoMeter;

impl Meter for NoMeter {
    fn measure(&self, f: &mut dyn FnMut()) -> usize {
        f();
        0
    }
}

/// The bounds a case is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes a read may request per byte of input.
    pub alloc_per_byte: usize,
    /// Bytes a read may request on top of that, whatever the input.
    pub alloc_floor: usize,
    /// How long a case may take before it counts as a hang.
    pub case_timeout: Duration,
}

/// The defaults are calibrated, not guessed: the reference reader's largest single read over the whole
/// corpus (22,620 reads, 20 bases) requested 6,522 bytes, so a floor of 16 KiB is 2.5 times that, and
/// the per-byte allowance covers the copy a reader makes of each block it parses.
impl Default for Limits {
    fn default() -> Limits {
        Limits { alloc_per_byte: 8, alloc_floor: 16 << 10, case_timeout: Duration::from_secs(10) }
    }
}

/// Why a case failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The reader panicked.
    Panicked(String),
    /// The reader did not finish in time.
    Hung,
    /// The reader accepted a hostile file.
    Accepted,
    /// The reader refused, naming a rule the case does not allow.
    WrongRule {
        /// What it said.
        got: String,
        /// What the case allows.
        want: Vec<String>,
    },
    /// The reader refused a file it had to accept.
    Rejected(Refusal),
    /// The reader requested more memory than the input justifies.
    Allocated {
        /// Bytes requested.
        bytes: usize,
        /// Bytes allowed.
        allowed: usize,
    },
    /// The base the mutation was made from was not accepted, so the case proves nothing.
    RepairRefused(Refusal),
    /// The reader panicked or hung on the base.
    RepairBroke(Box<Failure>),
}

/// The outcome of a run.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Cases that ran.
    pub ran: usize,
    /// Cases the reader does not provide for, by what they need.
    pub skipped: BTreeMap<Requires, usize>,
    /// The failures, by case name.
    pub failures: Vec<(String, Failure)>,
    /// The most any read requested.
    pub peak_allocation: usize,
    /// The most any read requested per byte of its input, in thousandths (a read of a 64 byte input
    /// that requested 6400 bytes is 100000).
    pub peak_milli_per_byte: usize,
}

impl Report {
    /// True when nothing failed.
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// The failures as a message for an assertion.
    pub fn summary(&self) -> String {
        let mut s = format!("{} ran, {} failed\n", self.ran, self.failures.len());
        for (name, f) in self.failures.iter().take(40) {
            s.push_str(&format!("  {name}: {f:?}\n"));
        }
        if self.failures.len() > 40 {
            s.push_str(&format!("  ... and {} more\n", self.failures.len() - 40));
        }
        s
    }

    /// The names of the failed cases.
    pub fn failed_names(&self) -> Vec<&str> {
        self.failures.iter().map(|(n, _)| n.as_str()).collect()
    }
}

fn message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_string())
}

/// Reads once, catching a panic.
fn guarded(reader: &dyn Reader, bytes: &[u8]) -> Result<Verdict, Failure> {
    catch_unwind(AssertUnwindSafe(|| reader.read(bytes))).map_err(|p| Failure::Panicked(message(&*p)))
}

type Cache = Arc<Mutex<HashMap<usize, Option<Failure>>>>;

/// Checks that the base reads, once per base.
fn repaired_reads(reader: &dyn Reader, bases: &[Base], base: usize, cache: &Cache) -> Option<Failure> {
    if let Some(done) = cache.lock().ok().and_then(|c| c.get(&base).cloned()) {
        return done;
    }
    let outcome = match guarded(reader, &bases[base].bytes) {
        Ok(Verdict::Accepted) => None,
        Ok(Verdict::Refused(r)) => Some(Failure::RepairRefused(r)),
        Err(f) => Some(Failure::RepairBroke(Box::new(f))),
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(base, outcome.clone());
    }
    outcome
}

struct Judged {
    failure: Option<Failure>,
    allocated: usize,
    len: usize,
}

fn judge(
    reader: &dyn Reader,
    meter: &dyn Meter,
    bases: &[Base],
    case: &Case,
    limits: &Limits,
    cache: &Cache,
) -> Judged {
    let bytes = case.bytes(&bases[case.base]);
    let mut verdict = None;
    let allocated = meter.measure(&mut || verdict = Some(guarded(reader, &bytes)));
    let verdict = verdict.expect("the closure ran");
    let fail = |failure| Judged { failure: Some(failure), allocated, len: bytes.len() };
    let verdict = match verdict {
        Ok(v) => v,
        Err(f) => return fail(f),
    };
    match (&case.expect, verdict) {
        (Expect::Refuse(_), Verdict::Accepted) => return fail(Failure::Accepted),
        (Expect::Refuse(rules), Verdict::Refused(r)) => {
            if !rules.contains(&r.rule.as_str()) {
                return fail(Failure::WrongRule {
                    got: r.rule,
                    want: rules.iter().map(|s| (*s).to_string()).collect(),
                });
            }
        }
        (Expect::Accept, Verdict::Refused(r)) => return fail(Failure::Rejected(r)),
        (Expect::Accept, Verdict::Accepted) => {}
    }
    let allowed = limits.alloc_floor + limits.alloc_per_byte * bytes.len();
    if allocated > allowed {
        return fail(Failure::Allocated { bytes: allocated, allowed });
    }
    Judged { failure: repaired_reads(reader, bases, case.base, cache), allocated, len: bytes.len() }
}

/// Keeps the panic message of a reader under test off stderr: the harness reports it, and a run of a
/// few hundred cases against a broken reader would print a few hundred backtraces. Panics anywhere
/// else (a test's own thread) still print.
fn quiet_hostile_threads() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !std::thread::current().name().is_some_and(|n| n.starts_with("hostile-")) {
                previous(info);
            }
        }));
    });
}

/// Runs every case the reader provides for. See the module documentation for what is checked.
pub fn run(
    reader: Arc<dyn Reader>,
    meter: Arc<dyn Meter>,
    bases: Arc<Vec<Base>>,
    cases: Arc<Vec<Case>>,
    limits: Limits,
) -> Report {
    quiet_hostile_threads();
    let provided = reader.provides();
    let cache: Cache = Arc::new(Mutex::new(HashMap::new()));
    let mut report = Report::default();
    for (index, case) in cases.iter().enumerate() {
        if case.requires != Requires::Parse && !provided.contains(&case.requires) {
            *report.skipped.entry(case.requires).or_insert(0) += 1;
            continue;
        }
        report.ran += 1;
        let (tx, rx) = channel();
        let (r, m, b, c, cc) = (reader.clone(), meter.clone(), bases.clone(), cases.clone(), cache.clone());
        std::thread::Builder::new()
            .name(format!("hostile-{index}"))
            .spawn(move || {
                let j = judge(&*r, &*m, &b, &c[index], &limits, &cc);
                let _ = tx.send((j.failure, j.allocated, j.len));
            })
            .expect("a thread");
        match rx.recv_timeout(limits.case_timeout) {
            Ok((failure, allocated, len)) => {
                report.peak_allocation = report.peak_allocation.max(allocated);
                report.peak_milli_per_byte = report.peak_milli_per_byte.max(allocated * 1000 / len.max(1));
                if let Some(f) = failure {
                    report.failures.push((case.name.clone(), f));
                }
            }
            Err(_) => report.failures.push((case.name.clone(), Failure::Hung)),
        }
    }
    report
}

/// The reference reader as a [`Reader`]. It provides [`Requires::Limits`] when it has a budget.
#[derive(Clone, Copy, Debug)]
pub struct ReferenceReader {
    /// What it is switched to do.
    pub options: crate::reference::Options,
}

impl ReferenceReader {
    /// Nesting checked and the default budget applied.
    pub fn new() -> ReferenceReader {
        ReferenceReader {
            options: crate::reference::Options {
                check_nesting: true,
                budget: Some(crate::reference::Budget::DEFAULT),
            },
        }
    }
}

impl Default for ReferenceReader {
    fn default() -> ReferenceReader {
        ReferenceReader::new()
    }
}

impl Reader for ReferenceReader {
    fn name(&self) -> String {
        "reference".to_string()
    }

    fn provides(&self) -> Vec<Requires> {
        if self.options.budget.is_some() {
            vec![Requires::Parse, Requires::Limits]
        } else {
            vec![Requires::Parse]
        }
    }

    fn read(&self, bytes: &[u8]) -> Verdict {
        match crate::reference::read_with(bytes, &self.options) {
            Ok(_) => Verdict::Accepted,
            Err(e) => Verdict::Refused(Refusal { rule: e.rule, detail: e.detail }),
        }
    }
}
