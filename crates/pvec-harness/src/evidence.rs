//! Which assertion each evidence class may support, and what a verdict is worth.

use crate::error::{ConformanceError, Result};
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

/// Whether a class may support an assertion. Known truth supports source intent, dual-oracle supports
/// converter agreement, real-world differential supports regression stability, and a quarantined
/// fixture supports nothing.
pub fn may_assert(class: EvidenceClass, assertion: Assertion) -> bool {
    matches!(
        (class, assertion),
        (EvidenceClass::KnownTruthExact, Assertion::SourceIntent)
            | (EvidenceClass::DualOracleEquivalent, Assertion::ConverterAgreement)
            | (EvidenceClass::RealWorldDifferential, Assertion::RegressionStability)
    )
}

impl Assertion {
    fn label(self) -> &'static str {
        match self {
            Assertion::SourceIntent => "source intent",
            Assertion::ConverterAgreement => "converter agreement",
            Assertion::RegressionStability => "regression stability",
        }
    }
}

impl Registry {
    /// Reads `(fixture, class text)` pairs as the manifest spells them. A spelling outside EPIC O's
    /// four classes is refused and names the fixture.
    pub fn from_manifest(entries: &[(&str, &str)]) -> Result<Registry> {
        let mut registry = Registry::default();
        for (id, text) in entries {
            let class = EvidenceClass::parse(text).ok_or_else(|| ConformanceError::Unclassified {
                fixture: (*id).to_string(),
                found: (*text).to_string(),
            })?;
            registry.classes.insert((*id).to_string(), class);
        }
        Ok(registry)
    }

    /// Takes the classes of the loaded corpus.
    pub fn from_corpus(corpus: &FixtureRegistry) -> Registry {
        Registry {
            classes: corpus.fixtures().iter().map(|f| (f.id.clone(), f.class)).collect(),
            registered: Vec::new(),
        }
    }

    /// The manifest's class for a fixture.
    pub fn class(&self, fixture: &str) -> Option<EvidenceClass> {
        self.classes.get(fixture).copied()
    }

    fn known(&self, fixture: &str) -> Result<EvidenceClass> {
        self.class(fixture).ok_or_else(|| ConformanceError::UnknownFixture(fixture.to_string()))
    }

    /// Checks a class someone wants to use for a fixture against the manifest's. The registry has no
    /// way to change a class, so the only answer to a different one is a refusal.
    pub fn declare_class(&self, fixture: &str, claimed: EvidenceClass) -> Result<EvidenceClass> {
        let manifest = self.known(fixture)?;
        if manifest != claimed {
            return Err(ConformanceError::ClassIsReadOnly {
                fixture: fixture.to_string(),
                manifest: manifest.name().to_string(),
                claimed: claimed.name().to_string(),
            });
        }
        Ok(manifest)
    }

    /// Registers an assertion for a fixture, refusing one its class cannot support.
    pub fn register(&mut self, fixture: &str, assertion: Assertion) -> Result<()> {
        let class = self.known(fixture)?;
        if !may_assert(class, assertion) {
            return Err(ConformanceError::AssertionNotSupported {
                fixture: fixture.to_string(),
                class: class.name().to_string(),
                assertion: assertion.label().to_string(),
            });
        }
        self.registered.push((fixture.to_string(), assertion));
        Ok(())
    }

    /// Records what a registered assertion observed.
    pub fn judge(&self, fixture: &str, assertion: Assertion, outcome: Outcome) -> Result<Verdict> {
        let class = self.known(fixture)?;
        if !self.registered.iter().any(|(f, a)| f == fixture && *a == assertion) {
            return Err(ConformanceError::NotRegistered {
                fixture: fixture.to_string(),
                assertion: assertion.label().to_string(),
            });
        }
        Ok(Verdict { fixture: fixture.to_string(), class, assertion: Some(assertion), outcome, counts: true })
    }

    /// Records that a quarantined fixture ran. It is the only way a quarantined fixture reports, and
    /// the verdict it returns never counts.
    pub fn judge_quarantined(&self, fixture: &str, outcome: Outcome) -> Result<Verdict> {
        let class = self.known(fixture)?;
        if class != EvidenceClass::Quarantined {
            return Err(ConformanceError::ClassIsReadOnly {
                fixture: fixture.to_string(),
                manifest: class.name().to_string(),
                claimed: EvidenceClass::Quarantined.name().to_string(),
            });
        }
        Ok(Verdict { fixture: fixture.to_string(), class, assertion: None, outcome, counts: false })
    }
}

/// Sets the uncounted verdicts aside and counts the rest.
pub fn tally(verdicts: &[Verdict]) -> Tally {
    let mut t = Tally { passed: 0, failed: 0, uncounted: 0 };
    for v in verdicts {
        match (v.counts, v.outcome) {
            (false, _) => t.uncounted += 1,
            (true, Outcome::Pass) => t.passed += 1,
            (true, Outcome::Fail) => t.failed += 1,
        }
    }
    t
}

/// One report row: the fixture, its class, the assertion and the verdict.
pub fn render(verdict: &Verdict) -> String {
    let what = verdict.assertion.map_or("executed only", Assertion::label);
    let result = match (verdict.counts, verdict.outcome) {
        (false, Outcome::Pass) => "PASS (not counted)",
        (false, Outcome::Fail) => "FAIL (not counted)",
        (true, Outcome::Pass) => "PASS",
        (true, Outcome::Fail) => "FAIL",
    };
    format!("{} [{}] {what}: {result}", verdict.fixture, verdict.class.name())
}

/// Phrases that make converter agreement a statement about the source. The same ban list the oracle
/// adapter applies to its own lines, plus the wordings a verdict row could drift into.
const SOURCE_CLAIMS: [&str; 4] =
    ["proves the original", "confirms the original", "true geometry", "agreement proves"];

/// Checks generated report text: a line carrying a verdict carries its class, and converter
/// agreement is never described as source truth.
pub fn check_report(text: &str) -> Result<()> {
    const CLASSES: [EvidenceClass; 4] = [
        EvidenceClass::KnownTruthExact,
        EvidenceClass::DualOracleEquivalent,
        EvidenceClass::RealWorldDifferential,
        EvidenceClass::Quarantined,
    ];
    let report = |line: &str, why: String| ConformanceError::Report { line: line.to_string(), why };
    for line in text.lines() {
        let has_verdict = line.contains(": PASS") || line.contains(": FAIL");
        if has_verdict && !CLASSES.iter().any(|c| line.contains(c.name())) {
            return Err(report(line, "carries a verdict but no evidence class".to_string()));
        }
        let lower = line.to_lowercase();
        if let Some(p) = SOURCE_CLAIMS.iter().find(|p| lower.contains(*p)) {
            return Err(report(line, format!("says `{p}`; converter agreement is not source truth")));
        }
        pvec_corpus::oracle::check_report_wording(line).map_err(|e| report(line, e.to_string()))?;
    }
    Ok(())
}
