//! The canonical artifact generator (issue P2.4).
//!
//! STUB: the red commit. Every entry point refuses, so the tests fail for the right reason.

use pvec_container::writer::{SortKey, WriteError};
use pvec_geometry::GeometryError;
use pvec_corpus::{Bundle, ExpectedMath, Fixture};
use std::fmt;
use std::path::{Path, PathBuf};

/// The generator's version, recorded in every artifact.
pub const GENERATOR_VERSION: &str = "pvec-gen/1";

/// The prefix of the producer id; the fixture id follows it.
pub const PRODUCER_PREFIX: &str = "pvec-gen:";

/// One generated artifact.
#[derive(Clone, Debug)]
pub struct Artifact {
    /// The fixture it was generated for.
    pub fixture_id: String,
    /// The corpus revision it came from.
    pub corpus_revision: String,
    /// The `.pvec` bytes.
    pub bytes: Vec<u8>,
    /// Record entity id and the `EntityId` it was given.
    pub ids: Vec<(String, u32)>,
}

impl Artifact {
    /// SHA-256 of the bytes, lowercase hex.
    pub fn sha256(&self) -> String {
        String::new()
    }
}

/// Why a fixture was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum Cause {
    /// The current capability set cannot represent the geometry exactly.
    MissingCapability {
        /// The registry capability that would be needed.
        capability: String,
        /// What about the geometry needs it.
        what: String,
    },
    /// The entry has no record.
    Quarantined,
    /// The manifest says no curve may be emitted for it.
    NotEmitted,
    /// Not something a capability would fix (units, a record that does not validate).
    Unsupported {
        /// What it was.
        what: String,
    },
    /// The writer refused.
    Write(WriteError),
    /// The semantic model refused (a numeric policy rule, for example).
    Geometry(GeometryError),
}

/// A refusal, naming the fixture, the entity and the cause.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    /// The fixture.
    pub fixture: String,
    /// The entity of the record, when it was one entity that could not be represented.
    pub entity: Option<String>,
    /// Why.
    pub cause: Cause,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: not implemented", self.fixture)
    }
}

impl std::error::Error for Refusal {}

fn not_implemented(fixture: &str) -> Refusal {
    Refusal {
        fixture: fixture.to_string(),
        entity: None,
        cause: Cause::Unsupported { what: "the generator is not implemented".to_string() },
    }
}

/// A record turned into writer calls, not yet written.
#[derive(Clone, Debug)]
pub struct Plan {
    _private: (),
}

impl Plan {
    /// How many entities.
    pub fn len(&self) -> usize {
        0
    }
    /// True when there are none.
    pub fn is_empty(&self) -> bool {
        true
    }
    /// Writes with the entities pushed in the order of `order`, a permutation of `0..len()`.
    pub fn write_in_order(&self, _order: &[usize]) -> Result<Vec<u8>, Refusal> {
        Err(not_implemented("plan"))
    }
    /// As [`Plan::write_in_order`] with one sort of the writer switched off.
    pub fn write_skipping(&self, _order: &[usize], _skip: SortKey) -> Result<Vec<u8>, Refusal> {
        Err(not_implemented("plan"))
    }
}

/// Maps one record onto writer calls.
pub fn plan(math: &ExpectedMath, _corpus_revision: &str) -> Result<Plan, Refusal> {
    Err(not_implemented(&math.fixture_id))
}

/// Generates the artifact for one record.
pub fn generate_record(math: &ExpectedMath, _corpus_revision: &str) -> Result<Artifact, Refusal> {
    Err(not_implemented(&math.fixture_id))
}

/// Generates the artifact for one registered fixture.
pub fn generate(fixture: &Fixture, _corpus_revision: &str) -> Result<Artifact, Refusal> {
    Err(not_implemented(&fixture.id))
}

/// What happened to one fixture.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// The fixture id.
    pub fixture: String,
    /// The artifact or the refusal.
    pub result: Result<Artifact, Refusal>,
}

/// Generates for every fixture of a bundle, in manifest order.
pub fn generate_all(_bundle: &Bundle) -> Vec<Outcome> {
    Vec::new()
}

/// What an artifact says about where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The fixture id.
    pub fixture_id: String,
    /// The corpus revision.
    pub corpus_revision: String,
    /// The generator version.
    pub generator_version: String,
}

/// Why provenance could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceError {
    /// The field that is missing or wrong.
    pub field: &'static str,
}

/// Reads the provenance out of an artifact; a missing field is an error.
pub fn provenance(_bytes: &[u8]) -> Result<Provenance, ProvenanceError> {
    Err(ProvenanceError { field: "everything" })
}

/// Writes every generated artifact as `<dir>/<fixture>.pvec` and returns the paths.
pub fn emit(_dir: &Path, _outcomes: &[Outcome]) -> std::io::Result<Vec<PathBuf>> {
    Ok(Vec::new())
}
