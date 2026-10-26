//! The thresholds and the verdicts of the geometry evaluation and tessellation benchmark (issue P8.4).

use std::collections::BTreeSet;

/// Failure if `point_at` on a degree-3 B-spline costs more than this many nanoseconds per evaluation.
pub const BSPLINE3_MAX_NS: f64 = 150.0;
/// Failure if halving the screen-error budget grows a conic's segment count by more than this factor.
pub const HALVING_MAX_GROWTH: f64 = 1.5;
/// Failure if evaluation at a 10^6 coordinate offset costs more than this multiple of the identity case.
pub const OFFSET_MAX_COST: f64 = 1.2;
/// Failure if `tessellate` allocates more than this many times per curve.
pub const MAX_ALLOCS_PER_TESSELLATION: f64 = 1.0;

/// What a threshold came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Measured and within the threshold.
    Pass,
    /// Measured and over the threshold. A result, not an error.
    Fail,
    /// Not measured, with the reason in the detail.
    NotMeasured,
}

/// One threshold on one platform.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    /// The threshold's id.
    pub id: &'static str,
    /// The platform the rows were measured on.
    pub platform: String,
    /// What it came to.
    pub status: Status,
    /// The numbers behind it, in words.
    pub detail: String,
}

/// One row of `geometry.csv`, one number per row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// `eval`, `tess`, `accuracy` or `summary`.
    pub table: String,
    /// The corpus entry the curves or the mix come from, or `synthetic`.
    pub entry: String,
    /// `linux-x86_64` or `linux-aarch64`.
    pub platform: String,
    /// The curve type.
    pub curve: String,
    /// The transform arm.
    pub transform: String,
    /// The screen-error budget in pixels, empty for rows that have none.
    pub budget_px: Option<f64>,
    /// The metric.
    pub metric: String,
    /// The number, or `None` when it could not be produced (the note says why).
    pub value: Option<f64>,
    /// Anything a reader needs.
    pub note: String,
}

/// The columns of `geometry.csv`, in order.
pub const HEADER: &str = "table,entry,platform,curve,transform,budget_px,metric,value,note";

/// The ids of the thresholds, in the order the document lists them.
pub const IDS: [&str; 5] = [
    "bspline3-point-at-150ns",
    "tessellate-one-allocation",
    "conic-halving-1.5x",
    "offset-1.2x-and-accuracy",
    "error-budget-never-exceeded",
];

impl Status {
    /// The word the document uses.
    pub fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::NotMeasured => "NOT MEASURED",
        }
    }
}

impl Row {
    /// The CSV line. A comma in the note would shift the columns, so it becomes a semicolon.
    pub fn line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{}",
            self.table,
            self.entry,
            self.platform,
            self.curve,
            self.transform,
            self.budget_px.map_or(String::new(), |v| v.to_string()),
            self.metric,
            self.value.map_or(String::new(), |v| v.to_string()),
            self.note.replace(',', ";")
        )
    }
}

/// Reads the rows of the CSV text.
pub fn parse_rows(csv: &str) -> Result<Vec<Row>, String> {
    let mut lines = csv.lines();
    if lines.next() != Some(HEADER) {
        return Err("the header is not the one the harness writes".into());
    }
    lines
        .enumerate()
        .map(|(n, line)| {
            let f: Vec<&str> = line.split(',').collect();
            if f.len() != 9 {
                return Err(format!("line {}: {} fields, expected 9", n + 2, f.len()));
            }
            let num = |i: usize| -> Result<Option<f64>, String> {
                if f[i].is_empty() {
                    Ok(None)
                } else {
                    f[i].parse().map(Some).map_err(|e| format!("line {}: column {i}: {e}", n + 2))
                }
            };
            Ok(Row {
                table: f[0].into(),
                entry: f[1].into(),
                platform: f[2].into(),
                curve: f[3].into(),
                transform: f[4].into(),
                budget_px: num(5)?,
                metric: f[6].into(),
                value: num(7)?,
                note: f[8].into(),
            })
        })
        .collect()
}

/// The verdicts for the rows, per platform: no comparison reads across two platforms.
pub fn verdicts(rows: &[Row]) -> Vec<Verdict> {
    let mut platforms: Vec<&str> = rows.iter().map(|r| r.platform.as_str()).collect();
    platforms.sort_unstable();
    platforms.dedup();
    let mut out = Vec::new();
    for platform in platforms {
        let here: Vec<&Row> = rows.iter().filter(|r| r.platform == platform).collect();
        let value = |table: &str,
                     entry: Option<&str>,
                     curve: &str,
                     transform: &str,
                     budget: Option<f64>,
                     metric: &str| {
            here.iter()
                .find(|r| {
                    r.table == table
                        && entry.is_none_or(|e| r.entry == e)
                        && r.curve == curve
                        && r.transform == transform
                        && r.budget_px == budget
                        && r.metric == metric
                })
                .and_then(|r| r.value)
        };
        let mut push = |id: &'static str, status: Status, detail: String| {
            out.push(Verdict { id, platform: platform.to_string(), status, detail })
        };
        let gap = |what: &str| (Status::NotMeasured, format!("not measured: {what}"));

        let (status, detail) = match (
            value("eval", Some("synthetic"), "bspline3", "identity", None, "point_at_ns"),
            value("eval", Some("synthetic"), "bspline3", "identity", None, "prototype_point_at_ns"),
        ) {
            (Some(ns), Some(proto)) => {
                let ok = ns <= BSPLINE3_MAX_NS && ns <= proto;
                (
                    if ok { Status::Pass } else { Status::Fail },
                    format!(
                        "point_at on the reference cubic {ns:.1} ns (limit {BSPLINE3_MAX_NS} ns), P1.3 prototype {proto:.1} ns"
                    ),
                )
            }
            _ => gap("the cubic or the P1.3 prototype baseline has no number on this platform"),
        };
        push(IDS[0], status, detail);

        let allocs: Vec<(&Row, f64)> = here
            .iter()
            .copied()
            .filter(|r| r.table == "tess" && r.metric == "allocs_per_tess")
            .filter_map(|r| r.value.map(|v| (r, v)))
            .collect();
        let (status, detail) = match allocs.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
            None => gap("no tessellation allocation count on this platform"),
            Some((r, v)) if *v > MAX_ALLOCS_PER_TESSELLATION => (
                Status::Fail,
                format!(
                    "{v} allocations for one tessellation of a {} at {} px, the most of {} rows (limit {MAX_ALLOCS_PER_TESSELLATION})",
                    r.curve,
                    r.budget_px.unwrap_or(0.0),
                    allocs.len()
                ),
            ),
            Some((_, v)) => (Status::Pass, format!("at most {v} allocations over {} rows", allocs.len())),
        };
        push(IDS[1], status, detail);

        let mut worst: Option<(f64, String)> = None;
        let mut compared = 0;
        for curve in ["circle", "ellipse"] {
            for (hi, lo) in [(1.0, 0.5), (0.5, 0.25)] {
                if let (Some(a), Some(b)) = (
                    value("tess", Some("synthetic"), curve, "identity", Some(hi), "segments_per_curve"),
                    value("tess", Some("synthetic"), curve, "identity", Some(lo), "segments_per_curve"),
                ) {
                    compared += 1;
                    let growth = b / a;
                    if worst.as_ref().is_none_or(|w| growth > w.0) {
                        worst = Some((
                            growth,
                            format!("{curve} from {hi} px to {lo} px: {a:.1} to {b:.1} segments"),
                        ));
                    }
                }
            }
        }
        let (status, detail) = match worst {
            None => gap("no conic segment counts at two budgets on this platform"),
            Some((g, what)) => (
                if g <= HALVING_MAX_GROWTH { Status::Pass } else { Status::Fail },
                format!(
                    "the largest growth over {compared} halvings is {g:.2}x, {what} (limit {HALVING_MAX_GROWTH}x)"
                ),
            ),
        };
        push(IDS[2], status, detail);

        let mut worst_cost: Option<(f64, String)> = None;
        let mut over_tolerance: Vec<String> = Vec::new();
        let mut compared = 0;
        let kinds: BTreeSet<(&str, &str)> =
            here.iter().filter(|r| r.table == "eval").map(|r| (r.entry.as_str(), r.curve.as_str())).collect();
        for (entry, curve) in kinds {
            if let (Some(far), Some(near)) = (
                value("eval", Some(entry), curve, "chain-offset", None, "point_at_ns"),
                value("eval", Some(entry), curve, "identity", None, "point_at_ns"),
            ) {
                compared += 1;
                let cost = far / near;
                if worst_cost.as_ref().is_none_or(|w| cost > w.0) {
                    worst_cost = Some((cost, format!("{entry} {curve}: {far:.1} ns against {near:.1} ns")));
                }
            }
        }
        for r in here.iter().filter(|r| r.table == "accuracy" && r.metric == "error") {
            if let (Some(e), Some(t)) =
                (r.value, value("accuracy", Some(&r.entry), &r.curve, &r.transform, None, "tolerance"))
            {
                compared += 1;
                if e > t {
                    over_tolerance.push(format!("{} (error {e:.3e} over {t:.3e})", r.curve));
                }
            }
        }
        let (status, detail) = match (compared, &worst_cost) {
            (0, _) | (_, None) => gap("no pair of offset and identity evaluation costs on this platform"),
            (_, Some((c, what))) => {
                let ok = *c <= OFFSET_MAX_COST && over_tolerance.is_empty();
                let tol = if over_tolerance.is_empty() {
                    "accuracy inside P1.2's tolerance everywhere".to_string()
                } else {
                    format!("outside the tolerance: {}", over_tolerance.join(", "))
                };
                (
                    if ok { Status::Pass } else { Status::Fail },
                    format!("the dearest offset costs {c:.2}x ({what}; limit {OFFSET_MAX_COST}x); {tol}"),
                )
            }
        };
        push(IDS[3], status, detail);

        let errors: Vec<(&Row, f64)> = here
            .iter()
            .copied()
            .filter(|r| r.table == "tess" && r.metric == "achieved_error_px")
            .filter_map(|r| r.value.map(|v| (r, v)))
            .collect();
        let over: Vec<String> = errors
            .iter()
            .filter(|(r, v)| r.budget_px.is_some_and(|b| *v > b))
            .map(|(r, v)| format!("{} at {} px reached {v:.6} px", r.curve, r.budget_px.unwrap_or(0.0)))
            .collect();
        let (status, detail) = if errors.is_empty() {
            gap("no achieved error on this platform")
        } else if over.is_empty() {
            (Status::Pass, format!("{} rows, none above its budget", errors.len()))
        } else {
            (
                Status::Fail,
                format!("{} of {} rows over the budget: {}", over.len(), errors.len(), over.join("; ")),
            )
        };
        push(IDS[4], status, detail);
    }
    out
}
