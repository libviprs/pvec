//! Extension governance (issue P3.5): the gate a custom geometry extension has to pass before it can be
//! registered.
//!
//! The rule it protects is the profile rule: PVec's only claim about the mathematics is that it is
//! aligned to named ISO 10303-42 constructs, so a new curve kind has to show that no suitable construct
//! exists. A proposal is a markdown document with fixed sections (see `docs/extensions/TEMPLATE.md`);
//! [`check_proposal`] reads it and names every artifact that is missing, one by one. The gate is
//! [`Registry::register_extension`], which refuses an extension while any artifact is absent.

use crate::{Capability, CapabilityId, Class, Registry, Requirement, StepBehaviour};
use core::fmt;

/// EPIC O's four evidence classes. The vocabulary belongs to EPIC O (`libviprs/pvec-gold-fixtures`) and
/// is spelled exactly; a fifth class is refused here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceClass {
    /// Synthetic source parameters known independently.
    KnownTruthExact,
    /// Both converters mathematically agree.
    DualOracleEquivalent,
    /// A regression fixture, not proof of the original source semantics.
    RealWorldDifferential,
    /// Runs, but can neither pass nor fail a gate.
    Quarantined,
}

impl EvidenceClass {
    /// Parses the exact spelling, such as `KNOWN_TRUTH_EXACT`.
    pub fn parse(text: &str) -> Option<EvidenceClass> {
        match text {
            "KNOWN_TRUTH_EXACT" => Some(EvidenceClass::KnownTruthExact),
            "DUAL_ORACLE_EQUIVALENT" => Some(EvidenceClass::DualOracleEquivalent),
            "REAL_WORLD_DIFFERENTIAL" => Some(EvidenceClass::RealWorldDifferential),
            "QUARANTINED" => Some(EvidenceClass::Quarantined),
            _ => None,
        }
    }

    /// The exact spelling.
    pub fn name(self) -> &'static str {
        match self {
            EvidenceClass::KnownTruthExact => "KNOWN_TRUTH_EXACT",
            EvidenceClass::DualOracleEquivalent => "DUAL_ORACLE_EQUIVALENT",
            EvidenceClass::RealWorldDifferential => "REAL_WORLD_DIFFERENTIAL",
            EvidenceClass::Quarantined => "QUARANTINED",
        }
    }
}

/// An artifact a proposal has to carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingArtifact {
    /// The proof that no suitable Part 42 representation exists, naming each construct considered.
    Part42Argument,
    /// The mathematical definition.
    MathematicalDefinition,
    /// Bounds and evaluation rules.
    BoundsAndEvaluation,
    /// Tessellation rules.
    TessellationRules,
    /// STEP behaviour.
    StepBehaviour,
    /// GIS behaviour, with a fidelity class where it is not exact.
    GisBehaviour,
    /// Conformance fixtures classified under EPIC O's evidence classes.
    Fixtures,
}

/// One thing wrong with a proposal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalProblem {
    /// A required artifact is absent, empty, a placeholder or too thin to be an argument.
    Missing(MissingArtifact),
    /// A fixture names a class outside EPIC O's four.
    UnknownFixtureClass(String),
    /// Every fixture is quarantined, and a quarantined fixture can neither pass nor fail a gate.
    OnlyQuarantinedFixtures,
    /// STEP behaviour is recorded as exact without a fixture that records a passing round trip.
    ExactStepWithoutRoundTrip,
}

/// A fixture listed by a proposal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionFixture {
    /// The fixture's name.
    pub name: String,
    /// Its evidence class.
    pub class: EvidenceClass,
    /// True when the proposal records `round-trip: passing` for it.
    pub round_trip_passing: bool,
}

/// What a complete proposal claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalSummary {
    /// STEP behaviour is recorded as exact.
    pub step_exact: bool,
    /// GIS behaviour is recorded as exact.
    pub gis_exact: bool,
    /// The fixtures, in document order.
    pub fixtures: Vec<ExtensionFixture>,
}

const APPROXIMATION: &str = "bounded approximation with error reported";
/// Fewest words in a prose section for it to count as an argument and not a label.
const MIN_PROSE_WORDS: usize = 8;
/// Fewest words in a STEP or GIS statement.
const MIN_STATEMENT_WORDS: usize = 6;

/// The body of the `## heading` section, if the heading exists.
fn section(text: &str, heading: &str) -> Option<String> {
    let mut found = false;
    let mut body = String::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if found {
                break;
            }
            found = h.trim() == heading;
            continue;
        }
        if found {
            body.push_str(line);
            body.push('\n');
        }
    }
    found.then_some(body)
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Prose that is not empty, not a placeholder and not a bare label.
fn is_real_prose(body: &str, min_words: usize) -> bool {
    !body.contains("TODO") && words(body) >= min_words
}

/// Reads a proposal and returns what it claims, or every problem with it.
///
/// The sections are the `## ` headings of `docs/extensions/TEMPLATE.md`: *Part 42 constructs
/// considered*, *Mathematical definition*, *Bounds and evaluation rules*, *Tessellation rules*,
/// *STEP behaviour*, *GIS behaviour* and *Conformance fixtures*. A section that is absent, empty, still
/// holds a `TODO`, or is only a word or two counts as missing.
pub fn check_proposal(text: &str) -> Result<ProposalSummary, Vec<ProposalProblem>> {
    let mut problems = Vec::new();
    let mut miss = |a: MissingArtifact| problems.push(ProposalProblem::Missing(a));

    // The Part 42 argument: at least one `- `construct`: why it fails` entry with a real reason.
    let part42 = section(text, "Part 42 constructs considered").unwrap_or_default();
    let has_argument = !part42.contains("TODO")
        && part42.lines().any(|l| {
            let l = l.trim();
            l.strip_prefix("- `")
                .and_then(|r| r.split_once("`:"))
                .is_some_and(|(name, reason)| !name.trim().is_empty() && words(reason) >= 3)
        });
    if !has_argument {
        miss(MissingArtifact::Part42Argument);
    }

    for (heading, artifact) in [
        ("Mathematical definition", MissingArtifact::MathematicalDefinition),
        ("Bounds and evaluation rules", MissingArtifact::BoundsAndEvaluation),
        ("Tessellation rules", MissingArtifact::TessellationRules),
    ] {
        if !section(text, heading).is_some_and(|b| is_real_prose(&b, MIN_PROSE_WORDS)) {
            miss(artifact);
        }
    }

    // STEP: a statement that is either "no exact representation; bounded approximation with error
    // reported" or an exact claim.
    let step = section(text, "STEP behaviour").filter(|b| is_real_prose(b, MIN_STATEMENT_WORDS));
    let step_exact = match &step {
        None => {
            miss(MissingArtifact::StepBehaviour);
            false
        }
        Some(b) => {
            let lower = b.to_lowercase();
            if lower.contains(APPROXIMATION) {
                false
            } else if lower.contains("exact") {
                true
            } else {
                miss(MissingArtifact::StepBehaviour);
                false
            }
        }
    };

    // GIS: the same, and a fidelity class wherever it is not exact.
    let gis = section(text, "GIS behaviour").filter(|b| is_real_prose(b, MIN_STATEMENT_WORDS / 2));
    let gis_exact = match &gis {
        None => {
            miss(MissingArtifact::GisBehaviour);
            false
        }
        Some(b) => {
            let lower = b.to_lowercase();
            let approximate = lower.contains("densif") || lower.contains("approximat");
            let exact = lower.contains("exact") && !approximate;
            if !exact {
                let class = lower.split_once("fidelity class:").map(|(_, rest)| rest.trim().to_string());
                if class.is_none_or(|c| c.split_whitespace().next().is_none()) {
                    miss(MissingArtifact::GisBehaviour);
                }
            }
            exact
        }
    };

    // Fixtures: `- name: CLASS` with an optional `; round-trip: passing`.
    let mut fixtures = Vec::new();
    let mut unknown = Vec::new();
    for l in section(text, "Conformance fixtures").unwrap_or_default().lines() {
        let Some(item) = l.trim().strip_prefix("- ") else { continue };
        let Some((name, rest)) = item.split_once(':') else { continue };
        let mut parts = rest.split(';');
        let class_text = parts.next().unwrap_or("").trim();
        let passing = parts.any(|p| p.trim().replace(' ', "") == "round-trip:passing");
        match EvidenceClass::parse(class_text) {
            Some(class) => fixtures.push(ExtensionFixture {
                name: name.trim().to_string(),
                class,
                round_trip_passing: passing,
            }),
            None => unknown.push(class_text.to_string()),
        }
    }
    let have_fixtures = !fixtures.is_empty() || !unknown.is_empty();
    if !have_fixtures {
        miss(MissingArtifact::Fixtures);
    }
    for u in unknown {
        problems.push(ProposalProblem::UnknownFixtureClass(u));
    }
    if !fixtures.is_empty() && fixtures.iter().all(|f| f.class == EvidenceClass::Quarantined) {
        problems.push(ProposalProblem::OnlyQuarantinedFixtures);
    }
    if step_exact && !fixtures.iter().any(|f| f.round_trip_passing) {
        problems.push(ProposalProblem::ExactStepWithoutRoundTrip);
    }

    if problems.is_empty() { Ok(ProposalSummary { step_exact, gis_exact, fixtures }) } else { Err(problems) }
}

/// Why an extension was not registered. Nothing is registered when this is returned.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtensionError {
    /// The entry's class is not [`Class::Extension`].
    NotAnExtension,
    /// The entry's id is not the next dense id of the registry.
    IdNotNext {
        /// The id it had to have.
        expected: CapabilityId,
    },
    /// The name is already taken.
    DuplicateName(&'static str),
    /// The entry names an ISO construct. An extension exists because none fits.
    ExtensionNamesConstruct(&'static str),
    /// The extension is required but lists no ADR.
    RequiredWithoutAdr(&'static str),
    /// The proposal is incomplete.
    Proposal(Vec<ProposalProblem>),
    /// The STEP behaviour in the entry disagrees with the proposal, or is not stated.
    StepClaimMismatch(&'static str),
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtensionError::NotAnExtension => write!(f, "the entry is not of class extension"),
            ExtensionError::IdNotNext { expected } => {
                write!(f, "an extension takes the next dense id, which is {expected}")
            }
            ExtensionError::DuplicateName(n) => write!(f, "the name {n} is already registered"),
            ExtensionError::ExtensionNamesConstruct(n) => {
                write!(f, "extension {n} names an ISO construct; an extension exists because none fits")
            }
            ExtensionError::RequiredWithoutAdr(n) => {
                write!(f, "extension {n} is required but lists no ADR; extensions are optional by default")
            }
            ExtensionError::Proposal(p) => write!(f, "the proposal is incomplete: {p:?}"),
            ExtensionError::StepClaimMismatch(n) => {
                write!(f, "extension {n}: the STEP behaviour in the entry disagrees with its proposal")
            }
        }
    }
}

impl std::error::Error for ExtensionError {}

impl Registry {
    /// The id the next registration must take: ids are dense, so it is one above the highest assigned.
    pub fn next_id(&self) -> CapabilityId {
        CapabilityId::new((self.entries.len() + self.retired.len()) as u32 + 1)
    }

    /// Registers an extension, or refuses it and changes nothing.
    ///
    /// The gate, in order: the entry is an extension; it has the next dense id and a new name; it names
    /// no ISO construct; it is optional unless it lists an ADR; its proposal is complete (every missing
    /// artifact is reported); and the STEP behaviour it registers agrees with the proposal, which may
    /// claim an exact form only with a fixture that records a passing round trip.
    pub fn register_extension(
        &mut self,
        capability: Capability,
        proposal: &str,
    ) -> Result<ProposalSummary, ExtensionError> {
        if capability.class != Class::Extension {
            return Err(ExtensionError::NotAnExtension);
        }
        if capability.id != self.next_id() {
            return Err(ExtensionError::IdNotNext { expected: self.next_id() });
        }
        if self.entries.iter().any(|c| c.name == capability.name) {
            return Err(ExtensionError::DuplicateName(capability.name));
        }
        if !capability.iso_construct.is_empty() {
            return Err(ExtensionError::ExtensionNamesConstruct(capability.name));
        }
        if capability.requirement == Requirement::Required && capability.adr.is_none_or(str::is_empty) {
            return Err(ExtensionError::RequiredWithoutAdr(capability.name));
        }
        let summary = check_proposal(proposal).map_err(ExtensionError::Proposal)?;
        let agrees = match capability.step {
            StepBehaviour::ExactForm => summary.step_exact,
            StepBehaviour::BoundedApproximation => !summary.step_exact,
            _ => false,
        };
        if !agrees {
            return Err(ExtensionError::StepClaimMismatch(capability.name));
        }
        self.entries.push(capability);
        Ok(summary)
    }
}
