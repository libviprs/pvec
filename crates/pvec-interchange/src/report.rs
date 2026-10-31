//! Per-export fidelity reports (issue P6.8).
//!
//! An export that only says "done" hides what a reviewer needs, so every export can hand back a
//! [`Report`]: the source artifact digest, the target and its version, how many entities landed in each
//! of the five fidelity classes, the full entity list of every class that is not `Exact`, the measured
//! approximation error, and every unsupported feature with its reason and capability id. The JSON and
//! the Markdown are two renderings of the same value, and the JSON is byte-identical for identical input.
//! `docs/INTERCHANGE.md` chapter 7 documents the schema.
//!
//! The ledger of an export may hold several records for one entity (the GML export writes one per
//! segment, and a tile set repeats an entity per tile). The report counts each entity once, in the
//! weakest class any of its records has, with the largest error any of them measured.

use crate::{Fidelity, FidelityKind, FidelityRecord, Reason, Representation};
use pvec_capabilities::{CapabilityId, Registry};
use std::collections::BTreeMap;
use std::fmt::Write;

/// The version of the report schema. Any change to the keys of the JSON document bumps it, and
/// `tests/report_schema_25.txt` records the keys of every version.
pub const SCHEMA_VERSION: &str = "1.1";

const SCHEMA_NAME: &str = "pvec-fidelity-report";

/// Why a report was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// The records cover a different number of entities than the artifact has, so an entity is missing
    /// from the report or the report invents one.
    EntityCount {
        /// The artifact's entity count.
        expected: u32,
        /// The distinct entities in the records.
        found: u32,
    },
    /// The target name or version is blank.
    BlankTarget,
    /// Two raster rows name the same raster, so the counts could not reconcile.
    DuplicateRaster(u32),
}

impl core::fmt::Display for ReportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReportError::EntityCount { expected, found } => {
                write!(f, "the artifact has {expected} entities but the records cover {found}")
            }
            ReportError::BlankTarget => write!(f, "the target name and version must not be blank"),
            ReportError::DuplicateRaster(id) => write!(f, "raster {id} appears twice in the raster rows"),
        }
    }
}

impl std::error::Error for ReportError {}

/// What an export did with one raster of the artifact (`docs/INTERCHANGE.md` chapter 8).
#[derive(Clone, Debug, PartialEq)]
pub enum RasterOutcome {
    /// The export carried the raster, tile bytes and placement unchanged.
    Exported,
    /// The export did not carry it; the reason says why and is never blank.
    Unsupported(Reason),
}

/// One raster of the artifact and what the export did with it. A raster is not an entity, so it has no
/// ledger record; this is its row in the report.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterRow {
    /// The raster's id in the artifact (`RasterId`).
    pub raster: u32,
    /// What happened to it.
    pub outcome: RasterOutcome,
}

impl RasterRow {
    /// The row of a raster that `target` does not carry, with the reason a reader of the report sees.
    pub fn left_out(raster: u32, target: &str) -> RasterRow {
        let text = format!("the {target} export carries no raster");
        RasterRow { raster, outcome: RasterOutcome::Unsupported(Reason(text)) }
    }

    /// The row of a raster that was refused, with the refusal as the reason (its text starts with the rule).
    pub fn refused(raster: u32, why: &impl std::fmt::Display) -> RasterRow {
        RasterRow { raster, outcome: RasterOutcome::Unsupported(Reason(why.to_string())) }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    entity: u32,
    fidelity: FidelityKind,
    capability: CapabilityId,
    capability_name: Option<&'static str>,
    construct: String,
    records: u32,
    error: Option<f64>,
    reason: Option<String>,
}

/// A fidelity report. Built once, rendered as JSON or Markdown.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    target: String,
    version: String,
    digest: [u8; 32],
    representation: &'static str,
    rows: Vec<Row>,
    rasters: Vec<RasterRow>,
}

impl Report {
    /// Builds the report of an export of `artifact_entities` entities.
    ///
    /// `records` are the records the export wrote, in any order. They have to cover exactly
    /// `artifact_entities` distinct entities, otherwise the counts could not reconcile and the report
    /// is refused.
    pub fn new<'a>(
        target: &str,
        target_version: &str,
        source_digest: [u8; 32],
        artifact_entities: u32,
        records: impl IntoIterator<Item = &'a FidelityRecord>,
    ) -> Result<Report, ReportError> {
        if target.trim().is_empty() || target_version.trim().is_empty() {
            return Err(ReportError::BlankTarget);
        }
        let registry = Registry::v1();
        let mut rows: BTreeMap<u32, Row> = BTreeMap::new();
        let (mut native, mut tessellated) = (false, false);
        for r in records {
            match r.target_construct().representation() {
                Representation::Native => native = true,
                Representation::Tessellated => tessellated = true,
            }
            let kind = r.fidelity().kind();
            let error = r.error_bound().map(|b| b.get());
            let reason = match r.fidelity() {
                Fidelity::Unsupported(reason) => Some(reason.as_str().to_string()),
                _ => None,
            };
            let id = r.entity_id().index();
            let row = rows.entry(id).or_insert_with(|| Row {
                entity: id,
                fidelity: kind,
                capability: r.source_capability(),
                capability_name: registry.get(r.source_capability()).map(|c| c.name),
                construct: r.target_construct().name().to_string(),
                records: 0,
                error: None,
                reason: None,
            });
            row.records += 1;
            row.error = match (row.error, error) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            // The weakest record names the entity's class, and the first of equals wins.
            if kind > row.fidelity || (kind == row.fidelity && row.reason.is_none() && reason.is_some()) {
                row.fidelity = kind;
                row.capability = r.source_capability();
                row.capability_name = registry.get(r.source_capability()).map(|c| c.name);
                row.construct = r.target_construct().name().to_string();
            }
            row.reason = row.reason.take().or(reason);
        }
        let found = rows.len() as u32;
        if found != artifact_entities {
            return Err(ReportError::EntityCount { expected: artifact_entities, found });
        }
        let representation = match (native, tessellated) {
            (false, true) => "Tessellated",
            (true, true) => "Mixed",
            _ => "Native",
        };
        Ok(Report {
            target: target.to_string(),
            version: target_version.to_string(),
            digest: source_digest,
            representation,
            rows: rows.into_values().collect(),
            rasters: Vec::new(),
        })
    }

    /// Adds what the export did with the artifact's rasters, one row per raster in any order. The report
    /// then says how many rasters the artifact has, how many were carried and why each other one was not.
    /// A raster named twice is refused, since the counts would not reconcile.
    pub fn with_rasters(
        mut self,
        rasters: impl IntoIterator<Item = RasterRow>,
    ) -> Result<Report, ReportError> {
        let mut rows: Vec<RasterRow> = rasters.into_iter().collect();
        rows.sort_by_key(|r| r.raster);
        if let Some(w) = rows.windows(2).find(|w| w[0].raster == w[1].raster) {
            return Err(ReportError::DuplicateRaster(w[0].raster));
        }
        self.rasters = rows;
        Ok(self)
    }

    fn rasters_exported(&self) -> usize {
        self.rasters.iter().filter(|r| r.outcome == RasterOutcome::Exported).count()
    }

    fn count(&self, kind: FidelityKind) -> usize {
        self.rows.iter().filter(|r| r.fidelity == kind).count()
    }

    /// Entities that may claim each conformance class: structural (Exact, ExactReparameterized),
    /// geometric (those and EquivalentCompound) and display (BoundedApproximation).
    fn claim_counts(&self) -> [usize; 3] {
        let claims =
            |f: fn(crate::Claims) -> bool| self.rows.iter().filter(|r| f(r.fidelity.claims())).count();
        [claims(|c| c.structural), claims(|c| c.geometric), claims(|c| c.display)]
    }

    /// The largest error and the nearest-rank 99th percentile over the entities that were approximated,
    /// both measured, and how many entities that is.
    fn error_summary(&self) -> (usize, Option<f64>, Option<f64>) {
        let mut errors: Vec<f64> = self.rows.iter().filter_map(|r| r.error).collect();
        errors.sort_by(f64::total_cmp);
        let n = errors.len();
        if n == 0 {
            return (0, None, None);
        }
        let rank = (99 * n).div_ceil(100);
        (n, errors.last().copied(), Some(errors[rank - 1]))
    }

    fn digest_text(&self) -> String {
        let mut s = String::from("sha256:");
        for b in self.digest {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    /// The JSON document. Keys are written in a fixed order, numbers as the shortest decimal that reads
    /// back to the same `f64`, and nothing in it depends on a clock or on the order records arrived in.
    pub fn to_json(&self) -> String {
        let mut o = String::new();
        let [structural, geometric, display] = self.claim_counts();
        let _ = writeln!(o, "{{\n  \"schema\": {},", quote(SCHEMA_NAME));
        let _ = writeln!(o, "  \"schema_version\": {},", quote(SCHEMA_VERSION));
        let _ = writeln!(o, "  \"source_digest\": {},", quote(&self.digest_text()));
        let _ = writeln!(
            o,
            "  \"target\": {{\"name\": {}, \"version\": {}, \"representation\": {}}},",
            quote(&self.target),
            quote(&self.version),
            quote(self.representation)
        );
        o.push_str("  \"conformance\": {\n");
        for (i, (name, n)) in [("structural", structural), ("geometric", geometric), ("display", display)]
            .into_iter()
            .enumerate()
        {
            let comma = if i < 2 { "," } else { "" };
            let _ = writeln!(o, "    \"{name}\": {{\"claimed\": {}, \"entities\": {n}}}{comma}", n > 0);
        }
        o.push_str("  },\n  \"entities\": {\n");
        let _ = writeln!(o, "    \"total\": {},", self.rows.len());
        let by: Vec<String> =
            FidelityKind::ALL.iter().map(|k| format!("\"{}\": {}", k.name(), self.count(*k))).collect();
        let _ = writeln!(o, "    \"by_fidelity\": {{{}}},", by.join(", "));
        o.push_str("    \"rows\": [");
        for (i, r) in self.rows.iter().enumerate() {
            o.push_str(if i == 0 { "\n" } else { ",\n" });
            let _ = write!(
                o,
                "      {{\"entity\": {}, \"fidelity\": {}, \"capability_id\": {}, \"target_construct\": {}, \
                 \"records\": {}, \"measured_error\": {}}}",
                r.entity,
                quote(r.fidelity.name()),
                r.capability.get(),
                quote(&r.construct),
                r.records,
                number(r.error)
            );
        }
        o.push_str(if self.rows.is_empty() { "],\n" } else { "\n    ],\n" });
        let lists: Vec<String> = FidelityKind::ALL
            .iter()
            .filter(|k| **k != FidelityKind::Exact)
            .map(|k| {
                let ids: Vec<String> =
                    self.rows.iter().filter(|r| r.fidelity == *k).map(|r| r.entity.to_string()).collect();
                format!("\"{}\": [{}]", k.name(), ids.join(", "))
            })
            .collect();
        let _ = writeln!(o, "    \"non_exact\": {{{}}}", lists.join(", "));
        o.push_str("  },\n");
        let (n, max, p99) = self.error_summary();
        let _ = writeln!(
            o,
            "  \"approximation_error\": {{\"basis\": \"measured\", \"entities\": {n}, \"max\": {}, \"p99\": {}}},",
            number(max),
            number(p99)
        );
        o.push_str("  \"unsupported\": [");
        let unsupported: Vec<&Row> =
            self.rows.iter().filter(|r| r.fidelity == FidelityKind::Unsupported).collect();
        for (i, r) in unsupported.iter().enumerate() {
            o.push_str(if i == 0 { "\n" } else { ",\n" });
            let _ = write!(
                o,
                "    {{\"entity\": {}, \"capability_id\": {}, \"capability\": {}, \"target_construct\": {}, \
                 \"reason\": {}}}",
                r.entity,
                r.capability.get(),
                r.capability_name.map_or("null".to_string(), quote),
                quote(&r.construct),
                quote(r.reason.as_deref().unwrap_or(""))
            );
        }
        o.push_str(if unsupported.is_empty() { "],\n" } else { "\n  ],\n" });
        let _ = writeln!(
            o,
            "  \"rasters\": {{\"count\": {}, \"exported\": {}, \"unsupported\": [",
            self.rasters.len(),
            self.rasters_exported()
        );
        let left: Vec<String> = self
            .rasters
            .iter()
            .filter_map(|r| match &r.outcome {
                RasterOutcome::Unsupported(reason) => {
                    Some(format!("    {{\"raster\": {}, \"reason\": {}}}", r.raster, quote(reason.as_str())))
                }
                RasterOutcome::Exported => None,
            })
            .collect();
        if !left.is_empty() {
            o.push_str(&left.join(",\n"));
            o.push('\n');
        }
        o.push_str("  ]}\n}\n");
        o
    }

    /// The Markdown rendering of the same data.
    pub fn to_markdown(&self) -> String {
        let mut o = String::new();
        let [structural, geometric, display] = self.claim_counts();
        let _ = writeln!(o, "# Fidelity report: {} {}\n", self.target, self.version);
        let _ = writeln!(o, "Schema `{SCHEMA_NAME}` version `{SCHEMA_VERSION}`.\n");
        let _ = writeln!(o, "Source artifact: `{}`.\n", self.digest_text());
        let _ = writeln!(o, "Target representation: {}.\n", self.representation);
        o.push_str("## Conformance claims\n\n| class | claimed | entities |\n|---|---|---|\n");
        for (name, n) in [("structural", structural), ("geometric", geometric), ("display", display)] {
            let _ = writeln!(o, "| {name} | {} | {n} |", if n > 0 { "yes" } else { "no" });
        }
        let _ = writeln!(o, "\n## Entities\n\n{} entities.\n", self.rows.len());
        o.push_str("| fidelity | entities |\n|---|---|\n");
        for k in FidelityKind::ALL {
            let _ = writeln!(o, "| {} | {} |", k.name(), self.count(k));
        }
        let (n, max, p99) = self.error_summary();
        let _ = writeln!(o, "\n## Approximation error (measured)\n");
        match (max, p99) {
            (Some(max), Some(p99)) => {
                let _ = writeln!(o, "{n} approximated entities: maximum {max}, 99th percentile {p99}.");
            }
            _ => o.push_str("No entity was approximated.\n"),
        }
        o.push_str("\n## Per entity\n\n| entity | fidelity | capability | target construct | records | measured error |\n");
        o.push_str("|---|---|---|---|---|---|\n");
        for r in &self.rows {
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} | {} | {} |",
                r.entity,
                r.fidelity.name(),
                r.capability.get(),
                cell(&r.construct),
                r.records,
                r.error.map_or("-".to_string(), |e| e.to_string())
            );
        }
        let unsupported: Vec<&Row> =
            self.rows.iter().filter(|r| r.fidelity == FidelityKind::Unsupported).collect();
        if !unsupported.is_empty() {
            o.push_str(
                "\n## Unsupported\n\n| entity | capability id | capability | reason |\n|---|---|---|---|\n",
            );
            for r in unsupported {
                let _ = writeln!(
                    o,
                    "| {} | {} | {} | {} |",
                    r.entity,
                    r.capability.get(),
                    r.capability_name.unwrap_or("-"),
                    cell(r.reason.as_deref().unwrap_or(""))
                );
            }
        }
        let _ = writeln!(
            o,
            "\n## Rasters\n\n{} rasters, {} exported.\n",
            self.rasters.len(),
            self.rasters_exported()
        );
        let left: Vec<&RasterRow> =
            self.rasters.iter().filter(|r| r.outcome != RasterOutcome::Exported).collect();
        if !left.is_empty() {
            o.push_str("| raster | reason |\n|---|---|\n");
            for r in left {
                if let RasterOutcome::Unsupported(reason) = &r.outcome {
                    let _ = writeln!(o, "| {} | {} |", r.raster, cell(reason.as_str()));
                }
            }
        }
        o
    }
}

fn number(v: Option<f64>) -> String {
    v.map_or("null".to_string(), |v| v.to_string())
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
