//! The thresholds, the network model and the verdicts of the progressive coverage benchmark (issue P8.5).

use crate::layout_bench::{Row, Verdict};

/// Failure if the directory is not ready within this many seconds (10^6 architectural entry, 25 Mbit/s, 60 ms).
pub const DIRECTORY_MAX_S: f64 = 0.3;
/// Failure if complete Tier 0 takes longer than this on the 25 Mbit/s arm.
pub const TIER0_MAX_S: f64 = 1.5;
/// Failure if complete Tier 0 takes longer than this on the 5 Mbit/s arm.
pub const TIER0_SLOW_MAX_S: f64 = 4.0;
/// Failure if the worst preview error on screen at stage 2 is above this many pixels.
pub const PREVIEW_MAX_PX: f64 = 2.0;
/// Failure if full visible refinement takes longer than this many seconds.
pub const REFINE_MAX_S: f64 = 5.0;
/// Failure if a mid-load pan discards more than this share of the bytes already decoded.
pub const PAN_MAX_DISCARD: f64 = 0.2;
/// Failure if a stage time varies by more than this across three repeat runs.
pub const REPEAT_MAX_SPREAD: f64 = 0.2;
/// Range requests in flight at once, on every network arm.
pub const RANGE_CONCURRENCY: usize = 6;
/// The screen the recorded viewports are shown on, along the wider side, in pixels.
pub const SCREEN_PX: f64 = 1920.0;
/// The preview error budget the writer is given, in pixels at the fit zoom (`docs/STREAMING.md` section 4.3).
pub const PREVIEW_BUDGET_PX: f64 = 0.5;

/// A network arm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arm {
    /// The name in the rows.
    pub name: &'static str,
    /// The link rate in Mbit/s, or `None` for a local file (no network time at all).
    pub mbit_per_s: Option<f64>,
    /// The round trip before the first byte of a request, in milliseconds.
    pub rtt_ms: f64,
}

/// The three arms of the issue.
pub const ARMS: [Arm; 3] = [
    Arm { name: "local", mbit_per_s: None, rtt_ms: 0.0 },
    Arm { name: "25mbit-60ms", mbit_per_s: Some(25.0), rtt_ms: 60.0 },
    Arm { name: "5mbit-200ms", mbit_per_s: Some(5.0), rtt_ms: 200.0 },
];

/// The ids of the verdicts, in the order the document lists them.
pub const IDS: [&str; 9] = [
    "directory-300ms",
    "tier0-1500ms",
    "tier0-4s-5mbit",
    "coverage-complete",
    "preview-2px",
    "preview-identity",
    "refine-5s",
    "pan-discard-20pct",
    "repeat-spread-20pct",
];

/// When each request of `sizes` (bytes, in issue order) is complete, in seconds, if all are issued at
/// `start_s` over `arm` with [`RANGE_CONCURRENCY`] requests in flight. A request takes a free lane, waits
/// one round trip for its first byte, then moves its bytes over the one link, which carries one request's
/// bytes at a time in issue order. A local arm has no network time: every request is done at `start_s`.
pub fn completions(arm: &Arm, start_s: f64, sizes: &[u64]) -> Vec<f64> {
    let _ = (arm, start_s, sizes);
    todo!("the network model")
}

/// `entities represented / entities intersecting`; an empty viewport is complete.
pub fn coverage(in_view: u64, represented: u64) -> f64 {
    let _ = (in_view, represented);
    todo!("the coverage metric")
}

/// `(max - min) / median` of the runs.
pub fn spread(runs: &[f64]) -> f64 {
    let _ = runs;
    todo!("the repeat spread")
}

/// The verdicts for the rows, per platform.
pub fn verdicts(rows: &[Row]) -> Vec<Verdict> {
    let _ = rows;
    todo!("the verdicts")
}
