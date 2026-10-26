//! Which assertion each evidence class may support, and what a verdict is worth.

use crate::error::Result;
use pvec_capabilities::EvidenceClass;
use pvec_corpus::FixtureRegistry;
use std::collections::BTreeMap;

/// What a test claims about a fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assertion {
    /// The implementation reproduces the source parameters.
    SourceIntent,
    /// Two converters agree under the numeric policy.
    ConverterAgreement,
    /// Output has not moved from a recorded baseline.
    RegressionStability,
}

/// What a test observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The assertion held.
    Pass,
    /// It did not.
    Fail,
}

/// One recorded result, with the class it was earned under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The fixture id.
    pub fixture: String,
    /// The class, from the manifest.
    pub class: EvidenceClass,
    /// What was asserted. `None` for a quarantined fixture, which asserts nothing.
    pub assertion: Option<Assertion>,
    /// What was observed.
    pub outcome: Outcome,
    /// Whether a gate counts it. A quarantined fixture runs, but nothing it says counts.
    pub counts: bool,
}

/// What a gate sees after the uncounted verdicts are set aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    /// Counted passes.
    pub passed: usize,
    /// Counted failures.
    pub failed: usize,
    /// Verdicts that ran and do not count.
    pub uncounted: usize,
}

/// The fixtures of a run, each with its class from the manifest.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    classes: BTreeMap<String, EvidenceClass>,
    registered: Vec<(String, Assertion)>,
}

/// Whether a class may support an assertion.
pub fn may_assert(_class: EvidenceClass, _assertion: Assertion) -> bool {
    todo!("issue 36")
}

impl Registry {
    /// Reads `(fixture, class text)` pairs as the manifest spells them.
    pub fn from_manifest(_entries: &[(&str, &str)]) -> Result<Registry> {
        todo!("issue 36")
    }

    /// Takes the classes of the loaded corpus.
    pub fn from_corpus(_corpus: &FixtureRegistry) -> Registry {
        todo!("issue 36")
    }

    /// The manifest's class for a fixture.
    pub fn class(&self, _fixture: &str) -> Option<EvidenceClass> {
        todo!("issue 36")
    }

    /// Checks a class someone wants to use for a fixture against the manifest's.
    pub fn declare_class(&self, _fixture: &str, _claimed: EvidenceClass) -> Result<EvidenceClass> {
        todo!("issue 36")
    }

    /// Registers an assertion for a fixture, refusing one its class cannot support.
    pub fn register(&mut self, _fixture: &str, _assertion: Assertion) -> Result<()> {
        todo!("issue 36")
    }

    /// Records what a registered assertion observed.
    pub fn judge(&self, _fixture: &str, _assertion: Assertion, _outcome: Outcome) -> Result<Verdict> {
        todo!("issue 36")
    }

    /// Records that a quarantined fixture ran. It is the only way a quarantined fixture reports, and
    /// the verdict it returns never counts.
    pub fn judge_quarantined(&self, _fixture: &str, _outcome: Outcome) -> Result<Verdict> {
        todo!("issue 36")
    }
}

/// Sets the uncounted verdicts aside and counts the rest.
pub fn tally(_verdicts: &[Verdict]) -> Tally {
    todo!("issue 36")
}

/// One report row: the fixture, its class, the assertion and the verdict.
pub fn render(_verdict: &Verdict) -> String {
    todo!("issue 36")
}

/// Checks generated report text: a verdict carries its class, and agreement is never source truth.
pub fn check_report(_text: &str) -> Result<()> {
    todo!("issue 36")
}
