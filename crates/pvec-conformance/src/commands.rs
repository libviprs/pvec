//! The subcommands, each over the bytes of a file.

use crate::decode::decode;
use crate::json::Json;
use crate::{Outcome, REPORT_SCHEMA_VERSION, exit};
use pvec_capabilities::{Declared, Registry, Requirement, Version};
use pvec_components::{CrsKind, DefinitionFormat, FitMethod, Provenance, check_residual};
use pvec_container::directory::{BlockEntry, BlockKind, parse_directory};
use pvec_container::manifest::DirectoryPointer;
use pvec_container::negotiate::{
    Outcome as Negotiated, ReaderCapabilities, Refusal, negotiate, version_gaps,
};
use pvec_container::reader::{ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::{Header, Limits, Manifest};
use pvec_entity::EntityId;
use pvec_geometry::Curve;
use pvec_interchange::Fidelity;
use pvec_interchange::mvt::{TessError, tessellate as tessellate_curve};
use pvec_interchange::step::verify::{Disagreement, Report, Subject, Verdict, compare, requests};
use std::time::Instant;

fn u(n: impl Into<u64>) -> Json {
    Json::Uint(n.into())
}

fn s(x: impl Into<String>) -> Json {
    Json::Str(x.into())
}

fn list(items: impl IntoIterator<Item = Json>) -> Json {
    Json::Array(items.into_iter().collect())
}

fn ver(v: Version) -> Json {
    s(v.to_string())
}

fn requirement(r: Requirement) -> Json {
    s(match r {
        Requirement::Required => "required",
        Requirement::Optional => "optional",
    })
}

/// The report every subcommand wraps its result in.
fn envelope(
    command: &str,
    path: &str,
    size: usize,
    exit: u8,
    error: Option<(String, String)>,
    result: Json,
    mut human: String,
) -> Outcome {
    let status = match exit {
        exit::CONFORMS => "ok",
        exit::NONCONFORMING => "nonconforming",
        exit::UNSUPPORTED_CAPABILITY => "unsupported_capability",
        _ => "refused",
    };
    if let Some((code, message)) = &error {
        human = format!("{command} {path}: refused ({code}): {message}\n{human}");
    }
    let report = Json::object([
        ("report_schema_version", u(REPORT_SCHEMA_VERSION)),
        ("command", s(command)),
        ("input", Json::object([("path", s(path)), ("size_bytes", u(size as u64))])),
        ("exit_code", u(u64::from(exit))),
        ("status", s(status)),
        (
            "error",
            match &error {
                Some((code, message)) => Json::object([("code", s(code)), ("message", s(message))]),
                None => Json::Null,
            },
        ),
        ("result", result),
    ]);
    Outcome { exit, report, human, error_code: error.map(|(code, _)| code) }
}

fn refused(
    command: &str,
    path: &str,
    size: usize,
    exit: u8,
    code: String,
    message: String,
    result: Json,
) -> Outcome {
    envelope(command, path, size, exit, Some((code, message)), result, String::new())
}

/// A required capability the reader lacks, as opposed to a file that is simply invalid.
fn exit_of(e: &ReadError) -> u8 {
    match e {
        ReadError::Negotiation(
            Refusal::MissingRequired { .. }
            | Refusal::IncompatibleMajor { .. }
            | Refusal::ReaderTooOld { .. },
        ) => exit::UNSUPPORTED_CAPABILITY,
        _ => exit::REFUSED,
    }
}

fn read_refusal(command: &str, path: &str, size: usize, e: &ReadError, result: Json) -> Outcome {
    refused(command, path, size, exit_of(e), e.rule(), e.to_string(), result)
}

type Parts = (Header, Manifest, Vec<BlockEntry>);

/// The header, manifest and directory, parsed without negotiating, so a file that needs a capability
/// this build lacks can still be inspected.
fn parts(bytes: &[u8]) -> Result<Parts, (String, String)> {
    let len = bytes.len() as u64;
    let bad = |rule: &str, e: &dyn std::fmt::Display| (rule.to_string(), e.to_string());
    let header = Header::parse(&bytes[..bytes.len().min(4096)], len).map_err(|e| bad(e.rule(), &e))?;
    let slice = |offset: u64, length: u64| {
        let end = offset.checked_add(length)?;
        bytes.get(usize::try_from(offset).ok()?..usize::try_from(end).ok()?)
    };
    let manifest_bytes = slice(header.manifest_offset, header.manifest_length)
        .ok_or_else(|| ("LAYOUT-05".to_string(), "the manifest lies outside the file".to_string()))?;
    let manifest = Manifest::parse(manifest_bytes, &header, len).map_err(|e| bad(e.rule(), &e))?;
    let pointer = manifest
        .directories
        .iter()
        .find(|d| d.kind == DirectoryPointer::KIND_BLOCK)
        .ok_or_else(|| ("LAYOUT-03".to_string(), "no block directory pointer".to_string()))?;
    let table = slice(pointer.offset, pointer.length)
        .ok_or_else(|| ("LAYOUT-05".to_string(), "the directory lies outside the file".to_string()))?;
    let entries = parse_directory(table, u64::from(header.header_len), len).map_err(|e| bad(e.rule(), &e))?;
    Ok((header, manifest, entries))
}

fn open(bytes: &[u8]) -> Result<ContainerReader<MemorySource<Vec<u8>>>, ReadError> {
    ContainerReader::open(MemorySource::new(bytes.to_vec()), ReaderConfig::default())
}

fn kind_name(e: &BlockEntry) -> &'static str {
    match e.block_kind() {
        Some(BlockKind::Hot) => "hot",
        Some(BlockKind::Records) => "records",
        Some(BlockKind::Arena) => "arena",
        Some(BlockKind::Components) => "components",
        Some(BlockKind::SpatialDirectory) => "spatial_directory",
        Some(BlockKind::CellRefs) => "cell_refs",
        Some(BlockKind::Straddlers) => "straddlers",
        Some(BlockKind::Tier0) => "tier0",
        None => pvec_container::negotiate::CAPABILITY_OWNED_KINDS
            .iter()
            .find(|(k, _, _)| *k == e.kind)
            .map_or("unknown", |(_, name, _)| name),
    }
}

fn bounds(b: &pvec_container::Bounds) -> Json {
    Json::object([
        ("min", list(b.min.iter().map(|x| Json::Real(*x)))),
        ("max", list(b.max.iter().map(|x| Json::Real(*x)))),
    ])
}

fn declared(registry: &Registry, d: &Declared) -> Json {
    Json::object([
        ("id", u(d.id.get())),
        ("name", registry.get(d.id).map_or(Json::Null, |c| s(c.name))),
        ("requirement", requirement(d.requirement)),
        ("version", ver(d.version)),
    ])
}

/// What `inspect` and `validate` say about the calibration (ADR 0019, `docs/CALIBRATION.md`): the JSON and the
/// human lines. `residual` runs the CAL-11 check, which only `validate` does. A refused calibration is reported with
/// its rule and never fails the file.
fn calibration_report(bytes: &[u8], residual: bool) -> (Json, String) {
    let refused = |e: &ReadError| {
        let json =
            Json::object([("status", s("refused")), ("rule", s(e.rule())), ("message", s(e.to_string()))]);
        (json, format!("calibration refused ({}): {e}\n", e.rule()))
    };
    let c = match open(bytes).and_then(|r| r.calibration()) {
        Ok(Some(c)) => c,
        Ok(None) => return (Json::object([("status", s("absent"))]), "calibration: none\n".to_string()),
        Err(e) => return refused(&e),
    };
    let checked = match (&c.georeference, residual) {
        (Some(g), true) => match check_residual(g) {
            Ok(r) => Json::Real(r),
            Err(e) => return refused(&ReadError::Calibration(e)),
        },
        _ => Json::Null,
    };
    let mut human = String::new();
    let scale = c.scale.as_ref().map_or(Json::Null, |sc| {
        let provenance = match sc.provenance {
            Provenance::DeclaredUnit => "declared_unit",
            Provenance::PlanScale { .. } => "plan_scale",
            Provenance::Measured { .. } => "measured",
            Provenance::Georeference => "georeference",
            Provenance::Manual => "manual",
            _ => "unknown",
        };
        human.push_str(&format!(
            "calibration scale: {} m per source unit ({provenance})\n",
            sc.metres_per_source_unit
        ));
        Json::object([
            ("metres_per_source_unit", Json::Real(sc.metres_per_source_unit)),
            ("provenance", s(provenance)),
            ("uncertainty", sc.uncertainty.map_or(Json::Null, Json::Real)),
            ("note", s(&sc.note)),
        ])
    });
    let georeference = c.georeference.as_ref().map_or(Json::Null, |g| {
        let kind = match g.crs_kind {
            CrsKind::Projected => "projected",
            CrsKind::Geographic => "geographic",
            CrsKind::Engineering => "engineering",
            _ => "unknown",
        };
        let method = match g.method {
            FitMethod::Declared => "declared",
            FitMethod::Fitted => "fitted",
            FitMethod::Manual => "manual",
            _ => "unknown",
        };
        let definition = g.definition.as_ref().map_or(Json::Null, |d| {
            s(match d.format {
                DefinitionFormat::Wkt2 => "wkt2",
                DefinitionFormat::Projjson => "projjson",
                _ => "unknown",
            })
        });
        let id = if g.crs_id.is_empty() { "none" } else { g.crs_id.as_str() };
        human.push_str(&format!(
            "georeference: CRS {id} ({kind}, {method}, {} points)\n",
            g.control_points.len()
        ));
        Json::object([
            ("crs_id", s(&g.crs_id)),
            ("crs_kind", s(kind)),
            ("method", s(method)),
            ("source_to_crs", list(g.source_to_crs.iter().map(|x| Json::Real(*x)))),
            ("points", u(g.control_points.len() as u64)),
            ("residual_max", Json::Real(g.residual_max)),
            ("definition_format", definition),
        ])
    });
    let json = Json::object([
        ("status", s("ok")),
        ("scale", scale),
        ("georeference", georeference),
        ("external_id", s(&c.external_id)),
        ("residual", checked),
    ]);
    (json, human)
}

/// `inspect`: header, manifest and directory summary.
pub fn inspect(bytes: &[u8], path: &str) -> Outcome {
    let size = bytes.len();
    let (header, manifest, entries) = match parts(bytes) {
        Ok(p) => p,
        Err((code, message)) => {
            return refused("inspect", path, size, exit::REFUSED, code, message, Json::object([]));
        }
    };
    let mut by_kind: Vec<(&str, u64, u64, u64)> = Vec::new();
    for e in &entries {
        let name = kind_name(e);
        let entities = if matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records)) {
            u64::from(e.count)
        } else {
            0
        };
        match by_kind.iter_mut().find(|k| k.0 == name) {
            Some(k) => {
                k.1 += 1;
                k.2 += entities;
                k.3 += u64::from(e.stored_length);
            }
            None => by_kind.push((name, 1, entities, u64::from(e.stored_length))),
        }
    }
    by_kind.sort();
    let entity_total: u64 = by_kind.iter().map(|k| k.2).sum();
    let (calibration, calibration_lines) = calibration_report(bytes, false);
    let result = Json::object([
        (
            "header",
            Json::object([
                ("container_major", u(header.major)),
                ("container_minor", u(header.minor)),
                ("header_len", u(header.header_len)),
                ("block_checksum", s(format!("{:?}", header.block_checksum))),
                ("artifact_checksum", s(format!("{:?}", header.artifact_checksum))),
                ("manifest_offset", u(header.manifest_offset)),
                ("manifest_length", u(header.manifest_length)),
            ]),
        ),
        (
            "manifest",
            Json::object([
                ("unit", s(format!("{:?}", manifest.unit).to_lowercase())),
                ("metres_per_unit", Json::Real(manifest.unit.metres_per_unit())),
                ("bounds", bounds(&manifest.bounds)),
                ("min_reader", ver(manifest.min_reader)),
                ("capabilities", u(manifest.capabilities.len() as u64)),
                ("producer_id", s(&manifest.metadata.producer_id)),
                ("producer_version", s(&manifest.metadata.producer_version)),
                ("block_checksums", Json::Bool(manifest.block_checksums.is_some())),
                ("artifact_digest", Json::Bool(manifest.artifact_digest.is_some())),
            ]),
        ),
        (
            "directory",
            Json::object([
                ("blocks", u(entries.len() as u64)),
                ("entities", u(entity_total)),
                (
                    "by_kind",
                    list(by_kind.iter().map(|k| {
                        Json::object([
                            ("kind", s(k.0)),
                            ("blocks", u(k.1)),
                            ("entities", u(k.2)),
                            ("stored_bytes", u(k.3)),
                        ])
                    })),
                ),
            ]),
        ),
        ("calibration", calibration),
    ]);
    let mut human = format!(
        "{path}: container {}.{}, {} bytes, {} blocks, {entity_total} entities, {} capabilities, producer {} {}\n",
        header.major,
        header.minor,
        size,
        entries.len(),
        manifest.capabilities.len(),
        manifest.metadata.producer_id,
        manifest.metadata.producer_version
    );
    // the manifest says what a unit is, the calibration what it stands for: both lines, neither replaces the other
    human.push_str(&format!("unit: {} m per source unit (manifest)\n", manifest.unit.metres_per_unit()));
    human.push_str(&calibration_lines);
    envelope("inspect", path, size, exit::CONFORMS, None, result, human)
}

/// `validate`: format validity, security limits and checksums.
pub fn validate(bytes: &[u8], path: &str) -> Outcome {
    let size = bytes.len();
    let reader = match open(bytes) {
        Ok(r) => r,
        Err(e) => {
            return read_refusal(
                "validate",
                path,
                size,
                &e,
                Json::object([("format_valid", Json::Bool(false))]),
            );
        }
    };
    let checked = reader.validate().and_then(|v| reader.verify_artifact_digest().map(|d| (v, d)));
    let (v, digest) = match checked {
        Ok(x) => x,
        Err(e) => {
            return read_refusal(
                "validate",
                path,
                size,
                &e,
                Json::object([("format_valid", Json::Bool(false))]),
            );
        }
    };
    let l = Limits::DEFAULT;
    let written = |b: bool| s(if b { "verified" } else { "not_written" });
    // a raster that is refused is a raster skipped, with its rule: the geometry and the other rasters are not
    // affected, so the file still conforms
    let rasters: Vec<(u32, Option<ReadError>)> =
        reader.rasters().into_iter().map(|id| (id, reader.validate_raster(id).err())).collect();
    let skipped_rasters = rasters.iter().filter(|(_, e)| e.is_some()).count();
    // a refused calibration, CAL-11 included, is reported with its rule and leaves the file conforming
    let (calibration, calibration_lines) = calibration_report(bytes, true);
    let result = Json::object([
        ("format_valid", Json::Bool(true)),
        ("blocks", u(v.blocks as u64)),
        ("entities", u(v.entities)),
        ("skipped_blocks", list(v.skipped.iter().map(|b| u(b.0)))),
        (
            "rasters",
            list(rasters.iter().map(|(id, e)| {
                Json::object([
                    ("raster", u(*id)),
                    ("status", s(if e.is_some() { "skipped" } else { "ok" })),
                    ("rule", e.as_ref().map_or(Json::Null, |e| s(e.rule()))),
                    ("message", e.as_ref().map_or(Json::Null, |e| s(e.to_string()))),
                ])
            })),
        ),
        ("calibration", calibration),
        (
            "checks",
            Json::object([
                ("structure", s("pass")),
                ("limits", s("pass")),
                ("block_checksums", written(reader.manifest().block_checksums.is_some())),
                (
                    "artifact_digest",
                    written(matches!(digest, pvec_container::integrity::DigestStatus::Verified)),
                ),
            ]),
        ),
        (
            "limits",
            Json::object([
                ("max_artifact_bytes", u(l.max_artifact_bytes)),
                ("max_block_bytes", u(l.max_block_bytes)),
                ("max_blocks", u(l.max_blocks)),
                ("max_expansion_ratio", u(l.max_expansion_ratio)),
                ("max_entities_per_artifact", u(l.max_entities_per_artifact)),
                ("max_nesting_depth", u(l.max_nesting_depth)),
                ("max_alloc_bytes", u(l.max_alloc_bytes)),
            ]),
        ),
    ]);
    let mut human =
        format!("{path}: valid, {} blocks, {} entities, {} skipped\n", v.blocks, v.entities, v.skipped.len());
    for (id, e) in rasters.iter().filter_map(|(id, e)| e.as_ref().map(|e| (id, e))) {
        human.push_str(&format!("raster {id} skipped ({}): {e}\n", e.rule()));
    }
    if skipped_rasters > 0 {
        human.push_str(&format!(
            "{skipped_rasters} raster{} skipped\n",
            if skipped_rasters == 1 { "" } else { "s" }
        ));
    }
    if calibration_lines != "calibration: none\n" {
        human.push_str(&calibration_lines);
    }
    envelope("validate", path, size, exit::CONFORMS, None, result, human)
}

/// `dump-manifest`.
pub fn dump_manifest(bytes: &[u8], path: &str) -> Outcome {
    let size = bytes.len();
    let (header, manifest, _) = match parts(bytes) {
        Ok(p) => p,
        Err((code, message)) => {
            return refused("dump-manifest", path, size, exit::REFUSED, code, message, Json::object([]));
        }
    };
    let registry = Registry::v1();
    let hex = |d: &[u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let range =
        |r: &pvec_container::ByteRange| Json::object([("offset", u(r.offset)), ("length", u(r.length))]);
    let result = Json::object([
        ("container_version", s(format!("{}.{}", header.major, header.minor))),
        ("min_reader", ver(manifest.min_reader)),
        (
            "unit",
            Json::object([
                ("name", s(format!("{:?}", manifest.unit).to_lowercase())),
                ("metres_per_unit", Json::Real(manifest.unit.metres_per_unit())),
            ]),
        ),
        ("bounds", bounds(&manifest.bounds)),
        ("capabilities", list(manifest.capabilities.iter().map(|d| declared(&registry, d)))),
        (
            "directories",
            list(manifest.directories.iter().map(|d| {
                Json::object([("kind", u(d.kind)), ("offset", u(d.offset)), ("length", u(d.length))])
            })),
        ),
        ("block_checksums", manifest.block_checksums.as_ref().map_or(Json::Null, range)),
        ("artifact_digest", manifest.artifact_digest.as_ref().map_or(Json::Null, |d| s(hex(d)))),
        (
            "metadata",
            Json::object([
                ("producer_id", s(&manifest.metadata.producer_id)),
                ("producer_version", s(&manifest.metadata.producer_version)),
                ("corpus_revision", manifest.metadata.corpus_revision.as_ref().map_or(Json::Null, s)),
            ]),
        ),
    ]);
    let human = format!("{}\n", result.render_pretty());
    envelope("dump-manifest", path, size, exit::CONFORMS, None, result, human)
}

/// `capabilities`: what the file requires against what this build supports.
pub fn capabilities(bytes: &[u8], path: &str) -> Outcome {
    let size = bytes.len();
    let (_, manifest, entries) = match parts(bytes) {
        Ok(p) => p,
        Err((code, message)) => {
            return refused("capabilities", path, size, exit::REFUSED, code, message, Json::object([]));
        }
    };
    let reader = ReaderCapabilities::v1();
    let registry = Registry::v1();
    let is_required = |d: &Declared| {
        d.requirement == Requirement::Required
            || registry.get(d.id).is_some_and(|c| c.requirement == Requirement::Required)
    };
    let lacks = |d: &Declared| match reader.supports(d.id) {
        None => true,
        Some((major, _)) => major != d.version.major,
    };
    let unsupported: Vec<&Declared> =
        manifest.capabilities.iter().filter(|d| is_required(d) && lacks(d)).collect();
    let outcome = negotiate(&manifest, &entries, &reader);
    let mut skipped: Vec<&Declared> =
        manifest.capabilities.iter().filter(|d| !is_required(d) && lacks(d)).collect();
    if let Negotiated::OpenDegraded(p) = &outcome {
        for id in p.skipped_ids() {
            if let Some(d) = manifest.capabilities.iter().find(|d| d.id.get() == id) {
                if !skipped.iter().any(|x| x.id == d.id) && !unsupported.iter().any(|x| x.id == d.id) {
                    skipped.push(d);
                }
            }
        }
    }
    skipped.sort_by_key(|d| d.id);
    let gaps = version_gaps(&manifest, &reader);
    let (code, error) = match &outcome {
        Negotiated::Refuse(r) => {
            let e = ReadError::Negotiation(r.clone());
            (exit_of(&e), Some((e.rule(), e.to_string())))
        }
        _ => (exit::CONFORMS, None),
    };
    let result = Json::object([
        ("reader_version", ver(reader.version)),
        ("file_min_reader", ver(manifest.min_reader)),
        ("supported", Json::Bool(!matches!(outcome, Negotiated::Refuse(_)))),
        ("declared", list(manifest.capabilities.iter().map(|d| declared(&registry, d)))),
        ("unsupported_required", list(unsupported.iter().map(|d| declared(&registry, d)))),
        ("skipped_optional", list(skipped.iter().map(|d| declared(&registry, d)))),
        (
            "version_gaps",
            list(gaps.iter().map(|g| {
                Json::object([
                    ("id", u(g.id.get())),
                    ("file_version", ver(g.file)),
                    ("supported", s(format!("{}.{}", g.supported.0, g.supported.1))),
                ])
            })),
        ),
    ]);
    let ids = |v: &[&Declared]| v.iter().map(|d| d.id.get().to_string()).collect::<Vec<_>>().join(", ");
    let human = format!(
        "{path}: {}\nrequired and unsupported: {}\noptional and skipped: {}\nnewer minor than this build: {}\n",
        if code == exit::CONFORMS { "supported" } else { "not supported" },
        if unsupported.is_empty() { "none".to_string() } else { ids(&unsupported) },
        if skipped.is_empty() { "none".to_string() } else { ids(&skipped) },
        if gaps.is_empty() {
            "none".to_string()
        } else {
            gaps.iter().map(|g| g.id.get().to_string()).collect::<Vec<_>>().join(", ")
        }
    );
    envelope("capabilities", path, size, code, error, result, human)
}

fn disagreements(items: &[Disagreement]) -> Json {
    list(items.iter().map(|d| {
        Json::object([
            ("entity", u(d.entity.index())),
            ("pvec", s(&d.pvec)),
            ("step", s(&d.step)),
            ("sample", s(&d.sample)),
            ("tolerance", Json::Real(d.tolerance)),
        ])
    }))
}

/// One class of the verdict: `pass`, the failing word, or `not_assessed`, never a merged boolean.
fn class(failing: &str, checked: usize, found: &[Disagreement]) -> Json {
    let status = if !found.is_empty() {
        failing
    } else if checked > 0 {
        "pass"
    } else {
        "not_assessed"
    };
    Json::object([
        ("status", s(status)),
        ("checked", u(checked as u64)),
        ("disagreements", disagreements(found)),
    ])
}

fn verdict_json(v: &Verdict) -> Json {
    Json::object([
        ("structural_exactness", class("differs", v.structural_checked, &v.structural)),
        ("geometric_exactness", class("fail", v.geometric_checked, &v.geometric)),
        ("display_fidelity", class("fail", v.display_checked, &v.display)),
    ])
}

/// `compare-step`: the file against an independent evaluator's report on a STEP export of it.
///
/// `step` is the STEP file the evaluator read and `report` the text it printed (`tools/step-eval`).
/// With `want_requests` the result is the sample requests for the evaluator's second pass.
pub fn compare_step(
    bytes: &[u8],
    path: &str,
    step_path: &str,
    step: &[u8],
    report: &str,
    want_requests: bool,
) -> Outcome {
    let size = bytes.len();
    let command = "compare-step";
    let fail = |code: &str, message: String| {
        refused(command, path, size, exit::REFUSED, code.to_string(), message, Json::object([]))
    };
    if !step.trim_ascii_start().starts_with(b"ISO-10303-21;") {
        return fail(
            "STEP-FILE",
            format!("{step_path} does not start with ISO-10303-21;, so it is not a Part 21 file"),
        );
    }
    let reader = match open(bytes) {
        Ok(r) => r,
        Err(e) => return read_refusal(command, path, size, &e, Json::object([])),
    };
    let decoded = match decode(&reader) {
        Ok(d) => d,
        Err(e) => return read_refusal(command, path, size, &e, Json::object([])),
    };
    if let Some((id, why)) = decoded.unsupported.first() {
        return fail("STEP-UNSUPPORTED", format!("entity {id} cannot be compared: {why}"));
    }
    fn trims(c: &Curve) -> bool {
        match c {
            Curve::Trimmed(_) => true,
            Curve::Composite(k) => k.segments().iter().any(trims),
            _ => false,
        }
    }
    let mut subjects = Vec::new();
    for (id, curve) in &decoded.curves {
        let Ok(id) = EntityId::new(*id) else {
            return fail("STEP-UNSUPPORTED", format!("entity id {id} is out of range"));
        };
        subjects.push(Subject {
            id,
            curve,
            fidelity: if trims(curve) { Fidelity::ExactReparameterized } else { Fidelity::Exact },
        });
    }
    let parsed = match Report::parse(report) {
        Ok(r) => r,
        Err(e) => return fail("STEP-REPORT", e),
    };
    let step_json = Json::object([("path", s(step_path)), ("size_bytes", u(step.len() as u64))]);
    let evaluator = s(parsed.evaluator());
    if want_requests {
        return match requests(&subjects, &parsed) {
            Ok(r) => envelope(
                command,
                path,
                size,
                exit::CONFORMS,
                None,
                Json::object([
                    ("step_file", step_json),
                    ("evaluator", evaluator),
                    ("requests", s(r.clone())),
                ]),
                format!("{r}\n"),
            ),
            Err(e) => fail("STEP-REPORT", e),
        };
    }
    let verdict = match compare(&subjects, &parsed) {
        Ok(v) => v,
        Err(e) => {
            return fail(
                "STEP-REPORT",
                format!("{e}; run with --requests for the samples the evaluator has to answer"),
            );
        }
    };
    let code = if verdict.passed() { exit::CONFORMS } else { exit::NONCONFORMING };
    let result = Json::object([
        ("step_file", step_json),
        ("evaluator", evaluator),
        ("entities", u(subjects.len() as u64)),
        ("verdict", verdict_json(&verdict)),
        (
            "not_comparable",
            list(
                verdict
                    .structural_not_comparable
                    .iter()
                    .map(|(id, why)| Json::object([("entity", u(id.index())), ("reason", s(*why))])),
            ),
        ),
    ]);
    let human = format!(
        "{path} against {step_path}: structural {} disagreements of {} checks, geometric {} of {}, display {} of {}\n",
        verdict.structural.len(),
        verdict.structural_checked,
        verdict.geometric.len(),
        verdict.geometric_checked,
        verdict.display.len(),
        verdict.display_checked
    );
    envelope(command, path, size, code, None, result, human)
}

/// `tessellate`: display fidelity only.
pub fn tessellate(bytes: &[u8], path: &str, error_px: f64, units_per_px: f64) -> Outcome {
    let size = bytes.len();
    let command = "tessellate";
    let reader = match open(bytes) {
        Ok(r) => r,
        Err(e) => return read_refusal(command, path, size, &e, Json::object([])),
    };
    let decoded = match decode(&reader) {
        Ok(d) => d,
        Err(e) => return read_refusal(command, path, size, &e, Json::object([])),
    };
    let tolerance = error_px * units_per_px;
    let mut worst_px = 0.0f64;
    let mut failed = false;
    let mut entities = Vec::new();
    for (id, curve) in &decoded.curves {
        let (status, points, deviation) = match tessellate_curve(curve, tolerance) {
            Ok(t) => ("ok", t.points.len() as u64, t.deviation),
            Err(TessError::ToleranceNotMet { achieved }) => {
                failed = true;
                ("tolerance_not_met", 0, achieved)
            }
            Err(_) => ("not_tessellated", 0, 0.0),
        };
        worst_px = worst_px.max(deviation / units_per_px);
        entities.push(Json::object([
            ("id", u(*id)),
            ("status", s(status)),
            ("points", u(points)),
            ("deviation_world", Json::Real(deviation)),
            ("deviation_px", Json::Real(deviation / units_per_px)),
        ]));
    }
    let result = Json::object([
        ("error_budget_px", Json::Real(error_px)),
        ("units_per_pixel", Json::Real(units_per_px)),
        ("tolerance_world", Json::Real(tolerance)),
        (
            "display_fidelity",
            Json::object([
                ("status", s(if failed { "fail" } else { "pass" })),
                ("max_deviation_px", Json::Real(worst_px)),
            ]),
        ),
        ("entities", list(entities)),
        (
            "skipped",
            list(
                decoded
                    .unsupported
                    .iter()
                    .map(|(id, why)| Json::object([("id", u(*id)), ("reason", s(why))])),
            ),
        ),
    ]);
    let human = format!(
        "{path}: tessellated {} entities to {error_px} px, largest deviation {worst_px:.4} px, display fidelity {}\n",
        decoded.curves.len(),
        if failed { "not met" } else { "met" }
    );
    envelope(
        command,
        path,
        size,
        if failed { exit::NONCONFORMING } else { exit::CONFORMS },
        None,
        result,
        human,
    )
}

/// `bench-open`: how long opening takes.
pub fn bench_open(bytes: &[u8], path: &str, iterations: u32) -> Outcome {
    let size = bytes.len();
    let mut times = Vec::with_capacity(iterations as usize);
    for _ in 0..iterations {
        let start = Instant::now();
        let opened = open(bytes);
        let elapsed = start.elapsed().as_nanos();
        if let Err(e) = opened {
            return read_refusal("bench-open", path, size, &e, Json::object([]));
        }
        times.push(u64::try_from(elapsed).unwrap_or(u64::MAX));
    }
    times.sort_unstable();
    let (min, median, max) = (times[0], times[times.len() / 2], times[times.len() - 1]);
    let result = Json::object([
        ("iterations", u(iterations)),
        ("bytes", u(size as u64)),
        ("volatile", list([s("timings_ns")])),
        ("timings_ns", Json::object([("min", u(min)), ("median", u(median)), ("max", u(max))])),
    ]);
    let human = format!("{path}: opened {iterations} times, median {median} ns (min {min}, max {max})\n");
    envelope("bench-open", path, size, exit::CONFORMS, None, result, human)
}
