//! The thresholds and the verdicts of the PVec against STEP comparison (issue P8.2). Not implemented yet.

/// Failure if `.pvec` cold-open to directory-ready is not at least this many times faster than a full Part 21
/// parse on the 10^5-entity architectural entry.
pub const COLD_OPEN_MIN_SPEEDUP: f64 = 10.0;
/// Failure if one-viewport bytes exceed this fraction of the Part 21 file on the 10^6-entity architectural entry.
pub const VIEWPORT_MAX_FRACTION_OF_PART21: f64 = 0.20;
/// Failure if `.pvec` compressed size exceeds STEP-HDF's by more than this fraction on the same geometry.
pub const COMPRESSED_MAX_OVERHEAD: f64 = 0.10;
/// Failure if the `pvec-wasm` decode artefact exceeds this many KiB compressed.
pub const WASM_MAX_KIB: f64 = 500.0;

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
    /// The platform both sides were measured on.
    pub platform: String,
    /// What it came to.
    pub status: Status,
    /// The numbers behind it, in words.
    pub detail: String,
}

/// One row of `step_comparison.csv`: one arm on one corpus entry on one platform. A missing number is `None`
/// (an empty cell in the CSV) and the `note` says why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// The corpus entry, or `(artefact)` for the decode artefact of the arm.
    pub entry: String,
    /// `pvec`, `part21` or `step-hdf`.
    pub arm: String,
    /// `linux-x86_64` or `linux-aarch64`.
    pub platform: String,
    /// The host the row was measured on.
    pub host: String,
    /// Entities the arm holds.
    pub entities: Option<f64>,
    /// FNV-1a of the shape list all three arms were built from, in hex.
    pub geometry: String,
    /// Bytes of the encoded file as written.
    pub encoded_bytes: Option<f64>,
    /// Bytes under the arm's own compression.
    pub compressed_bytes: Option<f64>,
    /// Open to directory-ready, milliseconds.
    pub cold_open_ms: Option<f64>,
    /// Median over the 200 recorded viewports, milliseconds.
    pub viewport_p50_ms: Option<f64>,
    /// 95th percentile over the 200 recorded viewports, milliseconds.
    pub viewport_p95_ms: Option<f64>,
    /// The most bytes any one recorded viewport transferred.
    pub viewport_bytes_max: Option<f64>,
    /// Peak resident set, KiB.
    pub peak_rss_kib: Option<f64>,
    /// The decode artefact, gzip-compressed bytes (`(artefact)` rows only).
    pub wasm_gz_bytes: Option<f64>,
    /// Why anything is missing, and anything else a reader needs.
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
