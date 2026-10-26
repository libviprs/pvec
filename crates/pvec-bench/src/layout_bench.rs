//! The thresholds and the verdicts of the layout, index and compression benchmark (issue P8.3). Not
//! implemented yet.

/// Failure if the shipped SoA immediate path decodes fewer than this many times the entities per second of the
/// per-entity-record arm on the 10^6-entity architectural entry.
pub const SOA_MIN_SPEEDUP: f64 = 3.0;
/// Failure if the shipped index needs more than this multiple of the best alternative's bytes for the recorded
/// mid-zoom viewports (and any more range reads than that alternative).
pub const INDEX_MAX_BYTES_FACTOR: f64 = 1.25;
/// Failure if index plus duplicated references exceed this share of the file on any entry.
pub const INDEX_MAX_FILE_SHARE: f64 = 0.15;
/// Failure if the chosen block size is more than this fraction above the best measured arm on total viewport
/// bytes at the 10^5 and 10^6 scales.
pub const BLOCK_MAX_OFF_BEST: f64 = 0.10;
/// The link the compression crossover is judged at, in Mbit/s.
pub const LINK_MBIT_PER_S: f64 = 25.0;

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

/// One row of `layout.csv`, in the tidy form: one number per row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// `decode`, `index`, `block` or `compression`.
    pub table: String,
    /// The corpus entry.
    pub entry: String,
    /// `linux-x86_64` or `linux-aarch64`.
    pub platform: String,
    /// The arm.
    pub arm: String,
    /// The arm's parameter (viewport class, block size, codec level, scope), or empty.
    pub param: String,
    /// The metric.
    pub metric: String,
    /// The number, or `None` when the arm could not produce it (the note says why).
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
