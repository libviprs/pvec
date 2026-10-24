//! The fixture registry: each fixture named, classed, and tied to registry capabilities.

use crate::bundle::Bundle;
use crate::error::{CorpusError, Result};
use crate::expected::ExpectedMath;
use crate::json::Value;
use crate::sha256::hex_digest;
use pvec_capabilities::{CapabilityId, Class, EvidenceClass, Registry};

/// What a fixture is allowed to be cited for. The corpus proves reader mapping and nothing else:
/// there is no STEP oracle behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// Two open DWG readers decode the file to the authored mathematics.
    ReaderMapping,
    /// A converter's STEP output is exact against the authored mathematics. Needs a STEP oracle.
    StepExactness,
    /// Two independent converters agree. Needs the dual-oracle corpus.
    DualOracleAgreement,
}

/// One registered fixture.
#[derive(Clone, Debug)]
pub struct Fixture {
    /// The fixture id.
    pub id: String,
    /// The manifest group (`curves`, `transforms`, `blocks`).
    pub group: String,
    /// Its evidence class.
    pub class: EvidenceClass,
    /// The manifest's `evidence_scope`, for example `READER_MAPPING_ONLY`.
    pub scope: String,
    /// The manifest's `oracles` field, `none` for every fixture of r0-open.
    pub oracles: String,
    /// The capability names as the corpus spells them.
    pub corpus_capabilities: Vec<String>,
    /// The registry capabilities they resolve to.
    pub capabilities: Vec<CapabilityId>,
    /// Path of the DWG inside the bundle. `None` for a quarantined entry that has no file.
    pub dwg_path: Option<String>,
    /// The authored mathematics. `None` for a quarantined entry that has no record.
    pub expected: Option<ExpectedMath>,
    /// True when a mapper must not emit a curve for this fixture (`near-singular-001`).
    pub must_not_emit_curve: bool,
}

const READER_MAPPING_ONLY: &str = "READER_MAPPING_ONLY";

impl Fixture {
    /// Whether this fixture may be cited for the claim.
    pub fn supports(&self, claim: Claim) -> bool {
        let has_oracle = self.oracles != "none";
        match (self.class, claim) {
            (EvidenceClass::KnownTruthExact, Claim::ReaderMapping) => true,
            (EvidenceClass::KnownTruthExact, Claim::StepExactness) => {
                has_oracle && self.scope != READER_MAPPING_ONLY
            }
            (EvidenceClass::DualOracleEquivalent, Claim::DualOracleAgreement) => has_oracle,
            _ => false,
        }
    }
}

/// Every fixture of one bundle.
#[derive(Clone, Debug)]
pub struct FixtureRegistry {
    fixtures: Vec<Fixture>,
}

/// The corpus spells some capabilities differently from the registry. This is the whole table; a
/// name in neither column is an unknown capability. A registry name is also accepted as itself.
const CORPUS_NAMES: [(&str, &str); 10] = [
    ("line", "line"),
    ("polyline", "polyline"),
    ("circle", "circle"),
    ("ellipse", "ellipse"),
    ("trimmed_curve", "trimmed_curve"),
    ("composite_curve", "composite_curve"),
    ("bezier", "bezier_curve"),
    ("bspline", "b_spline_curve"),
    ("nurbs", "rational_b_spline_curve"),
    ("affine_transform", "affine_transform_2d"),
];

fn missing(what: impl Into<String>) -> CorpusError {
    CorpusError::Missing(what.into())
}

fn str_field<'a>(v: &'a Value, key: &str, owner: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| missing(format!("`{key}` of {owner}")))
}

/// True when the profile has a heading `## <n>. ...`.
fn profile_has_section(profile: &str, section: u32) -> bool {
    let prefix = format!("## {section}. ");
    profile.lines().any(|l| l.starts_with(&prefix))
}

impl FixtureRegistry {
    /// Registers every fixture of the bundle. `capabilities` is the PVec registry, `profile` the text
    /// of `docs/ISO_PROFILE.md`. A fixture with no valid evidence class, with a capability the
    /// registry or the profile does not have, or with a class its evidence cannot carry, is an error
    /// that names the fixture.
    pub fn load(bundle: &Bundle, capabilities: &Registry, profile: &str) -> Result<FixtureRegistry> {
        let manifest = bundle.manifest();
        let entries = manifest
            .get("fixtures")
            .and_then(Value::as_arr)
            .ok_or_else(|| missing("`fixtures` in the manifest"))?;
        let mut fixtures: Vec<Fixture> = Vec::with_capacity(entries.len());
        for entry in entries {
            fixtures.push(register_one(bundle, capabilities, profile, entry)?);
        }
        // Entries the corpus refused to carry are registered too, as QUARANTINED with no file, so the
        // registry says what is missing and why instead of leaving it out.
        if let Some(q) = bundle.file("quarantine.json") {
            let q = crate::json::parse(q)?;
            for list in ["excluded", "quarantined"] {
                for item in q.get(list).and_then(Value::as_arr).unwrap_or(&[]) {
                    let id = str_field(item, "id", "a quarantine entry")?.to_string();
                    if fixtures.iter().any(|f| f.id == id) {
                        return Err(CorpusError::Invalid {
                            what: format!("fixture `{id}`"),
                            why: "listed twice".to_string(),
                        });
                    }
                    fixtures.push(Fixture {
                        group: "quarantine".to_string(),
                        class: EvidenceClass::Quarantined,
                        scope: str_field(manifest, "evidence_scope", "the manifest")?.to_string(),
                        oracles: "none".to_string(),
                        corpus_capabilities: Vec::new(),
                        capabilities: Vec::new(),
                        dwg_path: item.get("file").and_then(Value::as_str).map(str::to_string),
                        expected: None,
                        must_not_emit_curve: false,
                        id,
                    });
                }
            }
        }
        Ok(FixtureRegistry { fixtures })
    }
    /// The fixtures, in manifest order, quarantined entries last.
    pub fn fixtures(&self) -> &[Fixture] {
        &self.fixtures
    }
    /// One fixture by id.
    pub fn get(&self, id: &str) -> Option<&Fixture> {
        self.fixtures.iter().find(|f| f.id == id)
    }
}

fn register_one(bundle: &Bundle, caps: &Registry, profile: &str, entry: &Value) -> Result<Fixture> {
    let id = str_field(entry, "id", "a fixture")?.to_string();
    if id.is_empty() {
        return Err(missing("a fixture id"));
    }

    // The class first: an unclassified fixture is refused before anything else about it matters.
    let class = match entry.get("evidence_class") {
        None => return Err(CorpusError::Unclassified { fixture: id, found: None }),
        Some(v) => {
            let text = v.as_str().unwrap_or("<not a string>");
            EvidenceClass::parse(text).ok_or_else(|| CorpusError::Unclassified {
                fixture: id.clone(),
                found: Some(text.to_string()),
            })?
        }
    };
    let scope = str_field(entry, "evidence_scope", &id)?.to_string();
    let oracles = str_field(entry, "oracles", &id)?.to_string();
    if matches!(class, EvidenceClass::DualOracleEquivalent | EvidenceClass::RealWorldDifferential)
        && oracles == "none"
    {
        return Err(CorpusError::ClassWithoutEvidence {
            fixture: id,
            class: class.name().to_string(),
            why: "the manifest records no oracle for it".to_string(),
        });
    }

    let mut corpus_capabilities = Vec::new();
    let mut resolved = Vec::new();
    for name in entry
        .get("capabilities")
        .and_then(Value::as_arr)
        .ok_or_else(|| missing(format!("`capabilities` of {id}")))?
    {
        let name = name.as_str().ok_or_else(|| missing(format!("a capability name of {id}")))?;
        let registry_name =
            CORPUS_NAMES.iter().find(|(corpus, _)| *corpus == name).map_or(name, |(_, registry)| *registry);
        let capability = caps.by_name(registry_name).ok_or_else(|| CorpusError::UnknownCapability {
            fixture: id.clone(),
            capability: name.to_string(),
        })?;
        if capability.class == Class::Geometry && !profile_has_section(profile, capability.profile_section) {
            return Err(CorpusError::CapabilityNotInProfile {
                fixture: id,
                capability: name.to_string(),
                registry_name: registry_name.to_string(),
            });
        }
        corpus_capabilities.push(name.to_string());
        resolved.push(capability.id);
    }

    let file = |kind: &str| -> Result<(String, &[u8], &str)> {
        let node = entry.get(kind).ok_or_else(|| missing(format!("`{kind}` of {id}")))?;
        let path = str_field(node, "path", &id)?.to_string();
        let sha = str_field(node, "sha256", &id)?;
        let bytes = bundle.file(&path).ok_or_else(|| missing(format!("{path}, which {id} lists")))?;
        Ok((path, bytes, sha))
    };
    let (dwg_path, dwg, dwg_sha) = file("dwg")?;
    let (_, record, record_sha) = file("expected")?;
    for (path, bytes, sha) in
        [(&dwg_path, dwg, dwg_sha), (&format!("fixtures/{id}/expected.math.json"), record, record_sha)]
    {
        let actual = hex_digest(bytes);
        if actual != sha {
            return Err(CorpusError::HashMismatch { what: path.clone(), pinned: sha.to_string(), actual });
        }
    }
    let expected = ExpectedMath::parse(record)?;
    if expected.fixture_id != id {
        return Err(CorpusError::Invalid {
            what: format!("fixture `{id}`"),
            why: format!("its record is for `{}`", expected.fixture_id),
        });
    }
    if expected.evidence_class != class {
        return Err(CorpusError::Invalid {
            what: format!("fixture `{id}` evidence class"),
            why: format!(
                "the manifest says {} and its authored record says {}",
                class.name(),
                expected.evidence_class.name()
            ),
        });
    }
    Ok(Fixture {
        group: str_field(entry, "group", &id)?.to_string(),
        class,
        scope,
        oracles,
        corpus_capabilities,
        capabilities: resolved,
        dwg_path: Some(dwg_path),
        expected: Some(expected),
        must_not_emit_curve: entry.get("downstream_disposition").is_some(),
        id,
    })
}
