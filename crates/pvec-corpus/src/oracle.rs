//! The dual STEP conversion oracle adapter (issue P2.2).
//!
//! For one fixture of EPIC O this loads `primary.step` (CAD Exchanger), `secondary.step` (ODA), the
//! implementation-neutral signature, and the evaluator record each STEP file went through. It never
//! runs a converter: the artifacts are produced elsewhere, on the native x86_64 host, and arrive here
//! as files whose digests are checked.
//!
//! Nothing here compares one converter's STEP with the other's. The STEP bytes are checked against
//! their digest and dropped; the only thing either path contributes is what the one independent
//! evaluator reported about it.

use crate::error::{CorpusError, Result};
use crate::json::{self, Value};
use crate::sha256::hex_digest;
use pvec_capabilities::EvidenceClass;
use std::collections::BTreeMap;

/// The two converters of EPIC O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Converter {
    /// CAD Exchanger, the primary path.
    CadExchanger,
    /// ODA, the secondary path.
    Oda,
}

/// Who produced the evaluator record: the independent STEP evaluator of EPIC O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatorIdentity {
    /// The tool name, always `step_eval`.
    pub name: String,
    /// The evaluator's own version.
    pub evaluator_version: String,
    /// The geometry kernel version it reads STEP with.
    pub occt_version: String,
    /// The numeric policy it stamped.
    pub numeric_policy_version: String,
}

/// One converter's output for a fixture, as recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepArtifact {
    /// Which converter wrote it.
    pub converter: Converter,
    /// The SDK product name.
    pub product_name: String,
    /// The SDK version.
    pub product_version: String,
    /// The host class generation ran on, `x86_64-native`.
    pub host_class: String,
    /// The machine that class stands for, for example `mars` or `nas`.
    pub machine: String,
    /// The driver that ran it, `tools/nas.sh`.
    pub driver: String,
    /// SHA-256 of the STEP file, already checked.
    pub step_sha256: String,
    /// What the independent evaluator reported for it.
    pub evaluator: EvaluatorIdentity,
}

/// Everything recorded for one fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleFixture {
    /// The fixture id.
    pub id: String,
    /// The class EPIC O's manifest gives it. Read, never derived.
    pub class: EvidenceClass,
    /// The primary (CAD Exchanger) artifact.
    pub primary: StepArtifact,
    /// The secondary (ODA) artifact, when there is one.
    pub secondary: Option<StepArtifact>,
    /// The signature document's `schema_version`.
    pub signature_schema: String,
}

/// A fixture that has both STEP outputs and one evaluator for both.
#[derive(Clone, Copy, Debug)]
pub struct DualOracle<'a> {
    /// The primary artifact.
    pub primary: &'a StepArtifact,
    /// The secondary artifact.
    pub secondary: &'a StepArtifact,
    /// The evaluator both went through.
    pub evaluator: &'a EvaluatorIdentity,
}

/// The files of the oracle set, keyed by path.
pub type Files = BTreeMap<String, Vec<u8>>;

const MANIFEST_SCHEMA: &str = "pvec/oracle-artifacts/1";
const VERIFICATION_SCHEMA: &str = "pvec-gold/verification/1";
const SIGNATURE_PREFIX: &str = "pvec-gold/signature/";
/// The only host class that counts: generation ran natively on x86_64, not emulated.
const HOST_CLASS: &str = "x86_64-native";
/// The only driver that counts: the build driver wraps its work in throw-away containers.
const DRIVER: &str = "tools/nas.sh";
const EVALUATOR: &str = "step_eval";

fn invalid(what: impl Into<String>, why: impl Into<String>) -> CorpusError {
    CorpusError::Invalid { what: what.into(), why: why.into() }
}

fn text<'a>(v: &'a Value, key: &str, owner: &str) -> Result<&'a str> {
    match v.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s),
        Some(_) => Err(invalid(format!("`{key}` of {owner}"), "is empty")),
        None => Err(CorpusError::Missing(format!("`{key}` of {owner}"))),
    }
}

fn object<'a>(v: &'a Value, key: &str, owner: &str) -> Result<&'a Value> {
    v.get(key)
        .filter(|o| o.as_obj().is_some())
        .ok_or_else(|| CorpusError::Missing(format!("`{key}` of {owner}")))
}

/// Reads the file a `{path, sha256}` node names out of `files` and checks its digest.
fn read_file<'a>(files: &'a Files, fixture: &str, node: &Value, owner: &str) -> Result<(&'a [u8], String)> {
    let path = text(node, "path", owner)?;
    let sha = text(node, "sha256", owner)?;
    let full = format!("{fixture}/{path}");
    let bytes =
        files.get(&full).ok_or_else(|| CorpusError::Missing(format!("{full}, which {owner} lists")))?;
    let actual = hex_digest(bytes);
    if actual != sha {
        return Err(CorpusError::HashMismatch { what: full, pinned: sha.to_string(), actual });
    }
    Ok((bytes, actual))
}

fn artifact(
    files: &Files,
    fixture: &str,
    node: &Value,
    expect: Converter,
    owner: &str,
) -> Result<StepArtifact> {
    let converter = match (text(node, "converter", owner)?, expect) {
        ("cadex", Converter::CadExchanger) => Converter::CadExchanger,
        ("oda", Converter::Oda) => Converter::Oda,
        (other, _) => {
            return Err(invalid(
                format!("`converter` of {owner}"),
                format!("`{other}` is not the converter of this path"),
            ));
        }
    };
    let product = object(node, "product", owner)?;
    let host = object(node, "host", owner)?;
    let host_class = text(host, "class", owner)?;
    if host_class != HOST_CLASS {
        return Err(invalid(
            format!("host class of {owner}"),
            format!("generation must run on `{HOST_CLASS}`, not `{host_class}`"),
        ));
    }
    let driver = text(host, "driver", owner)?;
    if driver != DRIVER {
        return Err(invalid(
            format!("driver of {owner}"),
            format!("generation must run through `{DRIVER}`, not `{driver}`"),
        ));
    }
    let (_, step_sha256) = read_file(files, fixture, object(node, "step", owner)?, owner)?;
    let (record, _) = read_file(files, fixture, object(node, "verification", owner)?, owner)?;
    let record = json::parse(record)?;
    if text(&record, "schema_version", owner)? != VERIFICATION_SCHEMA {
        return Err(invalid(
            format!("evaluator record of {owner}"),
            format!("only `{VERIFICATION_SCHEMA}` is understood"),
        ));
    }
    let tool = object(&record, "tool", owner)?;
    let evaluator = EvaluatorIdentity {
        name: text(tool, "name", owner)?.to_string(),
        evaluator_version: text(tool, "evaluator_version", owner)?.to_string(),
        occt_version: text(tool, "occt_version", owner)?.to_string(),
        numeric_policy_version: text(&record, "numeric_policy_version", owner)?.to_string(),
    };
    Ok(StepArtifact {
        converter,
        product_name: text(product, "name", owner)?.to_string(),
        product_version: text(product, "version", owner)?.to_string(),
        host_class: host_class.to_string(),
        machine: text(host, "machine", owner)?.to_string(),
        driver: driver.to_string(),
        step_sha256,
        evaluator,
    })
}

fn identity(e: &EvaluatorIdentity) -> String {
    format!("{} {} (kernel {}, {})", e.name, e.evaluator_version, e.occt_version, e.numeric_policy_version)
}

impl OracleFixture {
    /// Loads `<fixture>/oracle.json` and the files it lists out of `files`. Every listed file is
    /// checked against its digest. A class that claims dual-oracle evidence needs both STEP outputs,
    /// and both go through the same evaluator.
    pub fn load(files: &Files, fixture: &str) -> Result<OracleFixture> {
        let path = format!("{fixture}/oracle.json");
        let bytes = files.get(&path).ok_or_else(|| CorpusError::Missing(path.clone()))?;
        let manifest = json::parse(bytes)?;
        if text(&manifest, "schema_version", &path)? != MANIFEST_SCHEMA {
            return Err(invalid(
                format!("schema_version of {path}"),
                format!("only `{MANIFEST_SCHEMA}` is understood"),
            ));
        }
        let id = text(&manifest, "fixture_id", &path)?;
        if id != fixture {
            return Err(invalid(path, format!("it is the manifest of `{id}`, not `{fixture}`")));
        }
        let class_text = text(&manifest, "evidence_class", &path)?;
        let class = EvidenceClass::parse(class_text).ok_or_else(|| CorpusError::Unclassified {
            fixture: fixture.to_string(),
            found: Some(class_text.to_string()),
        })?;

        let (signature, _) = read_file(files, fixture, object(&manifest, "signature", &path)?, &path)?;
        let signature = json::parse(signature)?;
        let signature_schema = text(&signature, "schema_version", "the signature")?.to_string();
        if !signature_schema.starts_with(SIGNATURE_PREFIX)
            || text(&signature, "fixture_id", "the signature")? != fixture
        {
            return Err(invalid(
                "the signature",
                format!("not a `{SIGNATURE_PREFIX}*` document for `{fixture}`"),
            ));
        }

        let primary = artifact(
            files,
            fixture,
            object(&manifest, "primary", &path)?,
            Converter::CadExchanger,
            "the primary artifact",
        )?;
        let secondary = match manifest.get("secondary") {
            Some(node) => Some(artifact(files, fixture, node, Converter::Oda, "the secondary artifact")?),
            None => None,
        };
        match &secondary {
            Some(s) if s.evaluator != primary.evaluator => {
                return Err(CorpusError::EvaluatorMismatch {
                    fixture: fixture.to_string(),
                    primary: identity(&primary.evaluator),
                    secondary: identity(&s.evaluator),
                });
            }
            None if class == EvidenceClass::DualOracleEquivalent => {
                return Err(CorpusError::DualOracleUnavailable {
                    fixture: fixture.to_string(),
                    route: "REAL_WORLD_DIFFERENTIAL or QUARANTINED".to_string(),
                });
            }
            _ => {}
        }
        if primary.evaluator.name != EVALUATOR {
            return Err(invalid(
                "the evaluator record",
                format!("written by `{}`, not `{EVALUATOR}`", primary.evaluator.name),
            ));
        }
        Ok(OracleFixture { id: fixture.to_string(), class, primary, secondary, signature_schema })
    }

    /// The dual-oracle evidence, or the refusal that says why there is none.
    pub fn dual(&self) -> Result<DualOracle<'_>> {
        match &self.secondary {
            Some(secondary) => {
                Ok(DualOracle { primary: &self.primary, secondary, evaluator: &self.primary.evaluator })
            }
            None => Err(CorpusError::DualOracleUnavailable {
                fixture: self.id.clone(),
                route: if self.class == EvidenceClass::Quarantined {
                    "QUARANTINED"
                } else {
                    "REAL_WORLD_DIFFERENTIAL"
                }
                .to_string(),
            }),
        }
    }

    /// The report line for this fixture's converter agreement. It says what two converters agreeing
    /// is: agreement between converters, read by one evaluator, and nothing about the drawing's
    /// intent.
    pub fn report_line(&self) -> String {
        let evaluator = identity(&self.primary.evaluator);
        match &self.secondary {
            Some(s) => format!(
                "{}: {}: converter agreement between {} {} and {} {}, read by {evaluator}; \
                 this describes the converters, not the source drawing",
                self.id,
                self.class.name(),
                self.primary.product_name,
                self.primary.product_version,
                s.product_name,
                s.product_version
            ),
            None => format!(
                "{}: {}: one STEP output only, so no converter agreement is available; read by {evaluator}",
                self.id,
                self.class.name()
            ),
        }
    }
}

/// Phrases that turn converter agreement into a claim about the source. A ban list on purpose: a
/// negated mention is refused too, because the report emitter has no need to write these at all.
const SOURCE_CLAIMS: [&str; 8] = [
    "source truth",
    "ground truth",
    "true to the source",
    "proves the original",
    "proves the source",
    "confirms the source",
    "original intent",
    "original dwg intent",
];

/// Refuses report text that describes converter agreement as source truth.
pub fn check_report_wording(text: &str) -> Result<()> {
    let lower = text.to_lowercase();
    for phrase in SOURCE_CLAIMS {
        if lower.contains(phrase) {
            return Err(invalid(
                "report text",
                format!("says `{phrase}`; converter agreement is not source truth"),
            ));
        }
    }
    Ok(())
}
