//! The thresholds, the network model and the verdicts of the progressive coverage benchmark (issue P8.5).

use crate::layout_bench::{Row, Status, Verdict};

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
    let Some(mbit) = arm.mbit_per_s else { return vec![start_s; sizes.len()] };
    let bytes_per_s = mbit * 1_000_000.0 / 8.0;
    let mut lanes = [start_s; RANGE_CONCURRENCY];
    let mut link_free = start_s;
    sizes
        .iter()
        .map(|&len| {
            let lane = (0..RANGE_CONCURRENCY).min_by(|a, b| lanes[*a].total_cmp(&lanes[*b])).unwrap_or(0);
            let first_byte = lanes[lane] + arm.rtt_ms / 1000.0;
            let done = first_byte.max(link_free) + len as f64 / bytes_per_s;
            lanes[lane] = done;
            link_free = done;
            done
        })
        .collect()
}

/// `entities represented / entities intersecting`; an empty viewport is complete.
pub fn coverage(in_view: u64, represented: u64) -> f64 {
    if in_view == 0 { 1.0 } else { represented as f64 / in_view as f64 }
}

/// `(max - min) / median` of the runs.
pub fn spread(runs: &[f64]) -> f64 {
    let mut v = runs.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n => (v[n - 1] - v[0]) / if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 },
    }
}

/// The verdicts for the rows, per platform: no comparison reads across two platforms.
pub fn verdicts(rows: &[Row]) -> Vec<Verdict> {
    const ENTRY: &str = "architectural-1e6";
    const FAST: &str = "25mbit-60ms";
    const SLOW: &str = "5mbit-200ms";
    let mut platforms: Vec<&str> = rows.iter().map(|r| r.platform.as_str()).collect();
    platforms.sort_unstable();
    platforms.dedup();
    let mut out = Vec::new();
    for platform in platforms {
        let here: Vec<&Row> = rows.iter().filter(|r| r.platform == platform).collect();
        let value = |table: &str, entry: &str, arm: &str, param: &str, metric: &str| {
            here.iter()
                .find(|r| {
                    r.table == table
                        && r.entry == entry
                        && r.arm == arm
                        && r.param == param
                        && r.metric == metric
                })
                .and_then(|r| r.value)
        };
        let mut push = |id: &'static str, status: Status, detail: String| {
            out.push(Verdict { id, platform: platform.to_string(), status, detail })
        };
        let gap = |what: &str| (Status::NotMeasured, format!("not measured: {what}"));
        // one number against a limit, on the architectural million
        let limit = |table: &str, arm: &str, param: &str, metric: &str, max: f64, unit: &str| match value(
            table, ENTRY, arm, param, metric,
        ) {
            Some(v) => (
                if v <= max { Status::Pass } else { Status::Fail },
                format!("{ENTRY} on {arm}: {v:.4} {unit} (limit {max} {unit})"),
            ),
            None => gap(&format!("{ENTRY} has no {metric} ({param}) on {arm} on this platform")),
        };

        let (s, d) = limit("stage", FAST, "p95", "directory_s", DIRECTORY_MAX_S, "s");
        push(IDS[0], s, d);
        let (s, d) = limit("stage", FAST, "p95", "tier0_s", TIER0_MAX_S, "s");
        push(IDS[1], s, d);
        let (s, d) = limit("stage", SLOW, "p95", "tier0_s", TIER0_SLOW_MAX_S, "s");
        push(IDS[2], s, d);

        let covered: Vec<&&Row> = here
            .iter()
            .filter(|r| r.table == "stage" && r.metric == "coverage" && r.value.is_some())
            .collect();
        let short: Vec<String> = covered
            .iter()
            .filter(|r| r.value.is_some_and(|v| v < 1.0))
            .map(|r| format!("{} on {} ({:.6})", r.entry, r.arm, r.value.unwrap_or(0.0)))
            .collect();
        let (s, d) = match (covered.len(), short.is_empty()) {
            (0, _) => gap("no entry has a coverage row on this platform"),
            (n, true) => (Status::Pass, format!("{n} entry and arm rows, coverage 1.0 on every viewport")),
            (n, false) => (
                Status::Fail,
                format!("{} of {n} entry and arm rows are below 1.0: {}", short.len(), short.join("; ")),
            ),
        };
        push(IDS[3], s, d);

        let (s, d) = limit("stage", FAST, "max", "worst_preview_px", PREVIEW_MAX_PX, "px");
        push(IDS[4], s, d);

        let unnamed: Vec<&&Row> =
            here.iter().filter(|r| r.metric == "previews_without_identity" && r.value.is_some()).collect();
        let bad: Vec<String> = unnamed
            .iter()
            .filter(|r| r.value.is_some_and(|v| v > 0.0))
            .map(|r| format!("{} on {} ({})", r.entry, r.arm, r.value.unwrap_or(0.0)))
            .collect();
        let (s, d) = match (unnamed.len(), bad.is_empty()) {
            (0, _) => gap("no entry has a previews_without_identity row on this platform"),
            (n, true) => (Status::Pass, format!("{n} rows, every preview replaces a named entity")),
            (_, false) => (Status::Fail, format!("previews without a stable identity: {}", bad.join("; "))),
        };
        push(IDS[5], s, d);

        let (s, d) = limit("stage", FAST, "p95", "refine_full_s", REFINE_MAX_S, "s");
        push(IDS[6], s, d);

        let (s, d) = limit("interaction", FAST, "pan", "discard_share", PAN_MAX_DISCARD, "of decoded bytes");
        push(IDS[7], s, d);

        let mut cases: Vec<(&str, &str, &str)> = here
            .iter()
            .filter(|r| r.table == "repeat")
            .map(|r| (r.entry.as_str(), r.arm.as_str(), r.metric.as_str()))
            .collect();
        cases.sort_unstable();
        cases.dedup();
        let (mut judged, mut failed, mut worst) = (0, Vec::new(), 0.0f64);
        for (entry, arm, metric) in cases {
            let runs: Vec<f64> = here
                .iter()
                .filter(|r| r.table == "repeat" && r.entry == entry && r.arm == arm && r.metric == metric)
                .filter_map(|r| r.value)
                .collect();
            if runs.len() < 3 {
                continue;
            }
            judged += 1;
            let sp = spread(&runs);
            worst = worst.max(sp);
            if sp > REPEAT_MAX_SPREAD {
                failed.push(format!("{entry} {arm} {metric} ({:.1}%)", sp * 100.0));
            }
        }
        let (s, d) = match (judged, failed.is_empty()) {
            (0, _) => gap("no entry has three repeat runs of a stage on this platform"),
            (n, true) => (
                Status::Pass,
                format!(
                    "{n} stage times, the widest spread {:.1}% (limit {:.0}%)",
                    worst * 100.0,
                    REPEAT_MAX_SPREAD * 100.0
                ),
            ),
            (n, false) => (
                Status::Fail,
                format!(
                    "{} of {n} stage times vary by more than {:.0}%: {}",
                    failed.len(),
                    REPEAT_MAX_SPREAD * 100.0,
                    failed.join("; ")
                ),
            ),
        };
        push(IDS[8], s, d);
    }
    out
}
