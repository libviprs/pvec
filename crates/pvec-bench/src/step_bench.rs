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

/// The columns of `step_comparison.csv`, in order.
pub const HEADER: &str = "entry,arm,platform,host,entities,geometry,encoded_bytes,compressed_bytes,cold_open_ms,\
viewport_p50_ms,viewport_p95_ms,viewport_bytes_max,peak_rss_kib,wasm_gz_bytes,note";

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

fn cell(v: Option<f64>) -> String {
    v.map_or(String::new(), |x| x.to_string())
}

impl Row {
    /// The CSV line. A comma in the note would shift the columns, so it becomes a semicolon.
    pub fn line(&self) -> String {
        [
            self.entry.clone(),
            self.arm.clone(),
            self.platform.clone(),
            self.host.clone(),
            cell(self.entities),
            self.geometry.clone(),
            cell(self.encoded_bytes),
            cell(self.compressed_bytes),
            cell(self.cold_open_ms),
            cell(self.viewport_p50_ms),
            cell(self.viewport_p95_ms),
            cell(self.viewport_bytes_max),
            cell(self.peak_rss_kib),
            cell(self.wasm_gz_bytes),
            self.note.replace(',', ";"),
        ]
        .join(",")
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
            if f.len() != 15 {
                return Err(format!("line {}: {} fields, expected 15", n + 2, f.len()));
            }
            let num = |i: usize| -> Result<Option<f64>, String> {
                if f[i].is_empty() {
                    Ok(None)
                } else {
                    f[i].parse().map(Some).map_err(|e| format!("line {}: column {i}: {e}", n + 2))
                }
            };
            Ok(Row {
                entry: f[0].into(),
                arm: f[1].into(),
                platform: f[2].into(),
                host: f[3].into(),
                entities: num(4)?,
                geometry: f[5].into(),
                encoded_bytes: num(6)?,
                compressed_bytes: num(7)?,
                cold_open_ms: num(8)?,
                viewport_p50_ms: num(9)?,
                viewport_p95_ms: num(10)?,
                viewport_bytes_max: num(11)?,
                peak_rss_kib: num(12)?,
                wasm_gz_bytes: num(13)?,
                note: f[14].into(),
            })
        })
        .collect()
}

/// The ids of the thresholds, in the order the document lists them.
pub const IDS: [&str; 6] = [
    "cold-open-10x",
    "viewport-bytes-20pct",
    "compressed-size-110pct",
    "viewport-p95-vs-hdf",
    "wasm-500kib",
    "step-hdf-footprint-reported",
];

/// Every comparison is made between rows of one platform, and nothing here reads across two: a pair with an
/// arm64 side and an x64 side never meets, because each platform is judged from its own rows only.
pub fn verdicts(rows: &[Row]) -> Vec<Verdict> {
    let mut platforms: Vec<&str> = rows.iter().map(|r| r.platform.as_str()).collect();
    platforms.sort_unstable();
    platforms.dedup();
    let mut out = Vec::new();
    for platform in platforms {
        let here: Vec<&Row> = rows.iter().filter(|r| r.platform == platform).collect();
        let find = |entry: &str, arm: &str| here.iter().copied().find(|r| r.entry == entry && r.arm == arm);
        let mut push = |id: &'static str, status: Status, detail: String| {
            out.push(Verdict { id, platform: platform.to_string(), status, detail })
        };
        let gap = |what: &str| (Status::NotMeasured, format!("not measured: {what}"));

        let (status, detail) = match (find("architectural-1e5", "pvec"), find("architectural-1e5", "part21"))
        {
            (Some(Row { cold_open_ms: Some(a), .. }), Some(Row { cold_open_ms: Some(b), .. })) => {
                let ratio = b / a;
                let status = if ratio >= COLD_OPEN_MIN_SPEEDUP { Status::Pass } else { Status::Fail };
                (
                    status,
                    format!(
                        "architectural-1e5: Part 21 read {b:.1} ms against .pvec directory-ready {a:.3} ms, {ratio:.1}x (needs {COLD_OPEN_MIN_SPEEDUP}x)"
                    ),
                )
            }
            _ => gap("architectural-1e5 has no cold-open time for both .pvec and Part 21 on this platform"),
        };
        push(IDS[0], status, detail);

        let (status, detail) = match (find("architectural-1e6", "pvec"), find("architectural-1e6", "part21"))
        {
            (Some(Row { viewport_bytes_max: Some(v), .. }), Some(Row { encoded_bytes: Some(f), .. })) => {
                let frac = v / f;
                let status =
                    if frac <= VIEWPORT_MAX_FRACTION_OF_PART21 { Status::Pass } else { Status::Fail };
                (
                    status,
                    format!(
                        "architectural-1e6: the largest recorded viewport moves {v} bytes, {:.2}% of the {f} byte Part 21 file (limit {:.0}%)",
                        frac * 100.0,
                        VIEWPORT_MAX_FRACTION_OF_PART21 * 100.0
                    ),
                )
            }
            _ => gap("architectural-1e6 has no viewport bytes for .pvec or no Part 21 size on this platform"),
        };
        push(IDS[1], status, detail);

        // Pairs of (.pvec, STEP-HDF) rows on the same entry, for the two per-entry thresholds.
        let pairs: Vec<(&Row, &Row)> = here
            .iter()
            .copied()
            .filter(|r| r.arm == "pvec" && r.entry != "(artefact)")
            .filter_map(|p| find(&p.entry, "step-hdf").map(|h| (p, h)))
            .collect();
        let judge = |pick: fn(&Row) -> Option<f64>, over: fn(f64, f64) -> Option<f64>| {
            let (mut compared, mut failed): (usize, Vec<String>) = (0, Vec::new());
            for (p, h) in &pairs {
                if let (Some(a), Some(b)) = (pick(p), pick(h)) {
                    compared += 1;
                    if let Some(excess) = over(a, b) {
                        failed.push(format!("{} ({excess:.3})", p.entry));
                    }
                }
            }
            (compared, failed)
        };
        let (compared, failed) =
            judge(|r| r.compressed_bytes, |a, b| (a > b * (1.0 + COMPRESSED_MAX_OVERHEAD)).then_some(a / b));
        let (status, detail) = match (compared, failed.is_empty()) {
            (0, _) => gap("no entry has a compressed size for both .pvec and STEP-HDF on this platform"),
            (n, true) => (
                Status::Pass,
                format!(
                    "{n} entries compared, none above {:.0}% over STEP-HDF",
                    COMPRESSED_MAX_OVERHEAD * 100.0
                ),
            ),
            (n, false) => (
                Status::Fail,
                format!(
                    "{} of {n} entries over the limit, as .pvec over STEP-HDF: {}",
                    failed.len(),
                    failed.join(", ")
                ),
            ),
        };
        push(IDS[2], status, detail);

        let (compared, failed) = judge(|r| r.viewport_p95_ms, |a, b| (a > b).then_some(a / b));
        let (status, detail) = match (compared, failed.is_empty()) {
            (0, _) => gap("no entry has a viewport p95 for both .pvec and STEP-HDF on this platform"),
            (n, true) => (Status::Pass, format!("{n} entries compared, .pvec p95 never above STEP-HDF's")),
            (n, false) => (
                Status::Fail,
                format!(
                    "{} of {n} entries where .pvec p95 is above STEP-HDF's, as the ratio: {}",
                    failed.len(),
                    failed.join(", ")
                ),
            ),
        };
        push(IDS[3], status, detail);

        let (status, detail) = match find("(artefact)", "pvec").and_then(|r| r.wasm_gz_bytes) {
            Some(b) => {
                let kib = b / 1024.0;
                (
                    if kib <= WASM_MAX_KIB { Status::Pass } else { Status::Fail },
                    format!("decode artefact {kib:.1} KiB compressed (limit {WASM_MAX_KIB} KiB)"),
                )
            }
            None => gap("no decode artefact size for .pvec on this platform"),
        };
        push(IDS[4], status, detail);

        let (status, detail) = match find("(artefact)", "step-hdf") {
            Some(r) if r.wasm_gz_bytes.is_some() => (
                Status::Pass,
                format!("browser footprint {:.1} KiB compressed", r.wasm_gz_bytes.unwrap_or(0.0) / 1024.0),
            ),
            Some(r) if !r.note.trim().is_empty() => {
                (Status::Pass, format!("reported as a gap with its reason: {}", r.note))
            }
            Some(_) => (Status::Fail, "the STEP-HDF artefact row has neither a size nor a reason".into()),
            None => gap("no STEP-HDF artefact row on this platform"),
        };
        push(IDS[5], status, detail);
    }
    out
}

/// The nearest-rank percentile of a sample (`q` in 0 to 1), or `None` for an empty one.
pub fn percentile(values: &[f64], q: f64) -> Option<f64> {
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = (q * v.len() as f64).ceil() as usize;
    v.get(rank.max(1) - 1).copied()
}
