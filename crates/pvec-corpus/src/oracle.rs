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

use crate::error::Result;
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

impl OracleFixture {
    /// Loads `<fixture>/oracle.json` and the files it lists out of `files`.
    pub fn load(_files: &Files, _fixture: &str) -> Result<OracleFixture> {
        todo!("issue 35")
    }

    /// The dual-oracle evidence, or the refusal that says why there is none.
    pub fn dual(&self) -> Result<DualOracle<'_>> {
        todo!("issue 35")
    }

    /// The report line for this fixture's converter agreement.
    pub fn report_line(&self) -> String {
        todo!("issue 35")
    }
}

/// Refuses report text that describes converter agreement as source truth.
pub fn check_report_wording(_text: &str) -> Result<()> {
    todo!("issue 35")
}
