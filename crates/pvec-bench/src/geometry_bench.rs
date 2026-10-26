//! The thresholds and the verdicts of the geometry evaluation and tessellation benchmark (issue P8.4). Not
//! implemented yet.

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

/// The verdicts for the rows.
pub fn verdicts(_rows: &[Row]) -> Vec<Verdict> {
    Vec::new()
}

/// Reads the rows of the CSV text.
pub fn parse_rows(_csv: &str) -> Result<Vec<Row>, String> {
    Err("not implemented".into())
}
