//! The thresholds and the verdicts of the layout, index and compression benchmark (issue P8.3).

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

/// The columns of `layout.csv`, in order.
pub const HEADER: &str = "table,entry,platform,arm,param,metric,value,note";

/// The ids of the thresholds, in the order the document lists them.
pub const IDS: [&str; 5] = [
    "soa-3x-record",
    "index-vs-best-alternative",
    "index-share-15pct",
    "block-size-within-10pct",
    "compression-crossover-25mbit",
];

/// The entries the index is judged on: every entry of 10^5 entities and up.
pub const INDEX_ENTRIES: [&str; 7] = [
    "architectural-1e5",
    "architectural-1e6",
    "mep-1e5",
    "industrial-1e5",
    "huge-coordinate-1e5",
    "spline-heavy-1e5",
    "spline-heavy-1e6",
];

/// The scales the block size is judged at.
pub const BLOCK_ENTRIES: [&str; 2] = ["architectural-1e5", "architectural-1e6"];

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
            "{},{},{},{},{},{},{},{}",
            self.table,
            self.entry,
            self.platform,
            self.arm,
            self.param,
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
            if f.len() != 8 {
                return Err(format!("line {}: {} fields, expected 8", n + 2, f.len()));
            }
            let value = if f[6].is_empty() {
                None
            } else {
                Some(f[6].parse().map_err(|e| format!("line {}: value: {e}", n + 2))?)
            };
            Ok(Row {
                table: f[0].into(),
                entry: f[1].into(),
                platform: f[2].into(),
                arm: f[3].into(),
                param: f[4].into(),
                metric: f[5].into(),
                value,
                note: f[7].into(),
            })
        })
        .collect()
}

/// One MiB per second is 8.388608 Mbit per second.
const MIBS_TO_MBIT: f64 = 8.388608;

/// The verdicts for the rows, per platform: no comparison reads across two platforms.
pub fn verdicts(rows: &[Row]) -> Vec<Verdict> {
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
        let entry_names = |table: &str| -> Vec<&str> {
            let mut v: Vec<&str> =
                here.iter().filter(|r| r.table == table).map(|r| r.entry.as_str()).collect();
            v.sort_unstable();
            v.dedup();
            v
        };

        let e = "architectural-1e6";
        let (status, detail) = match (
            value("decode", e, "soa", "", "entities_per_s"),
            value("decode", e, "record", "", "entities_per_s"),
        ) {
            (Some(soa), Some(rec)) => {
                let ratio = soa / rec;
                (
                    if ratio >= SOA_MIN_SPEEDUP { Status::Pass } else { Status::Fail },
                    format!(
                        "{e}: SoA {soa:.3e} entities/s against {rec:.3e} for per-entity records, {ratio:.2}x (needs {SOA_MIN_SPEEDUP}x)"
                    ),
                )
            }
            _ => gap(
                "the 10^6 architectural entry has no SoA or no per-entity-record decode rate on this platform",
            ),
        };
        push(IDS[0], status, detail);

        let (mut judged, mut failed) = (0, Vec::new());
        for entry in INDEX_ENTRIES {
            let alts: Vec<&str> = here
                .iter()
                .filter(|r| {
                    r.table == "index"
                        && r.entry == entry
                        && r.arm != "shipped"
                        && r.param == "mid"
                        && r.metric == "ranges_mean"
                        && r.value.is_some()
                })
                .map(|r| r.arm.as_str())
                .collect();
            let best = |metric: &str| {
                alts.iter()
                    .filter_map(|a| value("index", entry, a, "mid", metric))
                    .fold(f64::INFINITY, f64::min)
            };
            if let (Some(sr), Some(sb)) = (
                value("index", entry, "shipped", "mid", "ranges_mean"),
                value("index", entry, "shipped", "mid", "bytes_mean"),
            ) && !alts.is_empty()
            {
                judged += 1;
                let (br, bb) = (best("ranges_mean"), best("bytes_mean"));
                if sr > br || sb > INDEX_MAX_BYTES_FACTOR * bb {
                    failed.push(format!("{entry} (shipped {sr:.2} ranges and {sb:.0} bytes against the best alternatives' {br:.2} and {bb:.0})"));
                }
            }
        }
        let (status, detail) = match (judged, failed.is_empty()) {
            (0, _) => gap("no entry has shipped and alternative mid-zoom ranges and bytes on this platform"),
            (n, true) => (
                Status::Pass,
                format!(
                    "{n} entries: never more ranges than the best alternative, never over {INDEX_MAX_BYTES_FACTOR}x its bytes"
                ),
            ),
            (n, false) => (
                Status::Fail,
                format!("{} of {n} entries over the limit: {}", failed.len(), failed.join("; ")),
            ),
        };
        push(IDS[1], status, detail);

        let (mut judged, mut failed) = (0, Vec::new());
        for entry in entry_names("index") {
            if let (Some(i), Some(d), Some(f)) = (
                value("index", entry, "shipped", "", "index_bytes"),
                value("index", entry, "shipped", "", "dup_bytes"),
                value("index", entry, "shipped", "", "file_bytes"),
            ) {
                judged += 1;
                let share = (i + d) / f;
                if share > INDEX_MAX_FILE_SHARE {
                    failed.push(format!("{entry} ({:.1}%)", share * 100.0));
                }
            }
        }
        let (status, detail) = match (judged, failed.is_empty()) {
            (0, _) => {
                gap("no entry has the shipped index, duplicated reference and file sizes on this platform")
            }
            (n, true) => (
                Status::Pass,
                format!("{n} entries, none above {:.0}% of the file", INDEX_MAX_FILE_SHARE * 100.0),
            ),
            (n, false) => (
                Status::Fail,
                format!(
                    "{} of {n} entries above {:.0}%: {}",
                    failed.len(),
                    INDEX_MAX_FILE_SHARE * 100.0,
                    failed.join(", ")
                ),
            ),
        };
        push(IDS[2], status, detail);

        let (mut judged, mut failed) = (0, Vec::new());
        for entry in BLOCK_ENTRIES {
            let best = here
                .iter()
                .filter(|r| {
                    r.table == "block"
                        && r.entry == entry
                        && r.arm.starts_with("rtree-")
                        && r.metric == "bytes_total"
                })
                .filter_map(|r| r.value)
                .fold(f64::INFINITY, f64::min);
            if let Some(shipped) = value("block", entry, "shipped", "", "bytes_total")
                && best.is_finite()
            {
                judged += 1;
                if shipped > best * (1.0 + BLOCK_MAX_OFF_BEST) {
                    failed.push(format!(
                        "{entry} (shipped {shipped:.0} bytes against {best:.0}, {:.1}% over)",
                        (shipped / best - 1.0) * 100.0
                    ));
                }
            }
        }
        let (status, detail) = match (judged, failed.is_empty()) {
            (0, _) => gap(
                "neither the 10^5 nor the 10^6 architectural entry has a shipped and a swept block total on this platform",
            ),
            (n, true) => (
                Status::Pass,
                format!(
                    "{n} scales: the shipped block size is within {:.0}% of the best arm on total viewport bytes",
                    BLOCK_MAX_OFF_BEST * 100.0
                ),
            ),
            (n, false) => (
                Status::Fail,
                format!(
                    "{} of {n} scales more than {:.0}% off the best arm: {}",
                    failed.len(),
                    BLOCK_MAX_OFF_BEST * 100.0,
                    failed.join("; ")
                ),
            ),
        };
        push(IDS[3], status, detail);

        let (mut judged, mut failed, mut worst) = (0, Vec::new(), f64::INFINITY);
        for entry in entry_names("compression") {
            for scope in ["block", "file"] {
                if let (Some(ratio), Some(d)) = (
                    value("compression", entry, "lz4", scope, "ratio"),
                    value("compression", entry, "lz4", scope, "decompress_mib_s"),
                ) {
                    judged += 1;
                    let crossover = (1.0 - ratio) * d * MIBS_TO_MBIT;
                    worst = worst.min(crossover);
                    if crossover < LINK_MBIT_PER_S {
                        failed.push(format!("{entry} per {scope} ({crossover:.1} Mbit/s)"));
                    }
                }
            }
        }
        let (status, detail) = match (judged, failed.is_empty()) {
            (0, _) => gap("no entry has LZ4 ratio and decompression rate on this platform"),
            (n, true) => (
                Status::Pass,
                format!(
                    "LZ4 wins on any link slower than its crossover; the lowest of {n} crossovers is {worst:.0} Mbit/s (link {LINK_MBIT_PER_S} Mbit/s)"
                ),
            ),
            (n, false) => (
                Status::Fail,
                format!(
                    "{} of {n} below the {LINK_MBIT_PER_S} Mbit/s link, as crossover: {}",
                    failed.len(),
                    failed.join(", ")
                ),
            ),
        };
        push(IDS[4], status, detail);
    }
    out
}
