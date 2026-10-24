//! Stub: the red tests come first.
#![allow(missing_docs, unused_variables)]

use pvec_capabilities::{CapabilityId, Registry};
use pvec_entity::EntityId;

pub const FORMAT_MAJOR: u32 = 1;
pub const FORMAT_MINOR: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorBound(f64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorBoundError { NotFinite, NotPositive }
impl ErrorBound {
    pub fn new(v: f64) -> Result<ErrorBound, ErrorBoundError> { todo!() }
    pub fn get(self) -> f64 { self.0 }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason(String);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyText;
impl Reason {
    pub fn new(t: &str) -> Result<Reason, EmptyText> { todo!() }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fidelity {
    Exact,
    ExactReparameterized,
    EquivalentCompound,
    BoundedApproximation(ErrorBound),
    Unsupported(Reason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FidelityKind { Exact, ExactReparameterized, EquivalentCompound, BoundedApproximation, Unsupported }
impl FidelityKind {
    pub fn name(self) -> &'static str { todo!() }
    pub fn parse(s: &str) -> Option<FidelityKind> { todo!() }
    pub fn claims(self) -> Claims { todo!() }
}
impl Fidelity {
    pub fn kind(&self) -> FidelityKind { todo!() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claims { pub structural: bool, pub geometric: bool, pub display: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation { Native, Tessellated }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetConstruct { name: String, representation: Representation }
impl TargetConstruct {
    pub fn new(name: &str, representation: Representation) -> Result<TargetConstruct, EmptyText> { todo!() }
    pub fn name(&self) -> &str { &self.name }
    pub fn representation(&self) -> Representation { self.representation }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError { NoCapability, TessellatedClaimedExact(FidelityKind) }

#[derive(Clone, Debug, PartialEq)]
pub struct FidelityRecord {
    entity_id: EntityId,
    source_capability: CapabilityId,
    target_construct: TargetConstruct,
    fidelity: Fidelity,
    note: String,
}
impl FidelityRecord {
    pub fn new(entity_id: EntityId, source_capability: CapabilityId, target_construct: TargetConstruct, fidelity: Fidelity, note: &str) -> Result<FidelityRecord, RecordError> { todo!() }
    pub fn entity_id(&self) -> EntityId { self.entity_id }
    pub fn source_capability(&self) -> CapabilityId { self.source_capability }
    pub fn target_construct(&self) -> &TargetConstruct { &self.target_construct }
    pub fn fidelity(&self) -> &Fidelity { &self.fidelity }
    pub fn error_bound(&self) -> Option<ErrorBound> { todo!() }
    pub fn note(&self) -> &str { &self.note }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FidelityLedger { records: Vec<FidelityRecord> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    BadHeader,
    UnsupportedMajor(u32),
    Truncated,
    MissingField { record: usize, field: &'static str },
    Invalid { line: usize, what: &'static str },
}

impl FidelityLedger {
    pub fn new() -> FidelityLedger { FidelityLedger::default() }
    pub fn record(&mut self, r: FidelityRecord) { self.records.push(r) }
    pub fn records(&self) -> &[FidelityRecord] { &self.records }
    pub fn count(&self, kind: FidelityKind) -> usize { todo!() }
    pub fn unregistered(&self, registry: &Registry) -> Vec<CapabilityId> { todo!() }
    pub fn to_text(&self) -> String { todo!() }
    pub fn from_text(text: &str) -> Result<FidelityLedger, ParseError> { todo!() }
}
