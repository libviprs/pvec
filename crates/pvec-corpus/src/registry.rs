//! The fixture registry: each fixture named, classed, and tied to registry capabilities.

use crate::bundle::Bundle;
use crate::error::Result;
use crate::expected::ExpectedMath;
use pvec_capabilities::{CapabilityId, EvidenceClass, Registry};

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

impl Fixture {
    /// Whether this fixture may be cited for the claim.
    pub fn supports(&self, _claim: Claim) -> bool {
        todo!("Fixture::supports")
    }
}

/// Every fixture of one bundle.
#[derive(Clone, Debug)]
pub struct FixtureRegistry {
    fixtures: Vec<Fixture>,
}

impl FixtureRegistry {
    /// Registers every fixture of the bundle. `capabilities` is the PVec registry, `profile` the text
    /// of `docs/ISO_PROFILE.md`. A fixture with no valid evidence class, with a capability the
    /// registry or the profile does not have, or with a class its evidence cannot carry, is an error
    /// that names the fixture.
    pub fn load(_bundle: &Bundle, _capabilities: &Registry, _profile: &str) -> Result<FixtureRegistry> {
        todo!("FixtureRegistry::load")
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
