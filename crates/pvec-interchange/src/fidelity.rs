//! The fidelity vocabulary, the record and the ledger.

use core::fmt;
use pvec_capabilities::{CapabilityId, Registry};
use pvec_entity::EntityId;

/// Major version of the serialized form. A reader refuses a larger major.
pub const FORMAT_MAJOR: u32 = 1;
/// Minor version of the serialized form. A larger minor only ever adds fields, which a reader of an
/// older minor skips.
pub const FORMAT_MINOR: u32 = 0;

const MAGIC: &str = "pvec-fidelity-ledger";

/// A finite, strictly positive error bound, in the units of the drawing.
///
/// This is the type-level half of the rule that a tessellated result cannot pass as exact:
/// [`Fidelity::BoundedApproximation`] takes one, and there is no way to make one from zero, a
/// negative number, NaN or an infinity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorBound(f64);

/// Why a number is not an [`ErrorBound`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorBoundError {
    /// NaN or an infinity.
    NotFinite,
    /// Zero (either sign) or negative.
    NotPositive,
}

impl ErrorBound {
    /// A bound, or the reason the number is not one.
    pub fn new(value: f64) -> Result<ErrorBound, ErrorBoundError> {
        if !value.is_finite() {
            Err(ErrorBoundError::NotFinite)
        } else if value <= 0.0 {
            Err(ErrorBoundError::NotPositive)
        } else {
            Ok(ErrorBound(value))
        }
    }

    /// The bound.
    pub fn get(self) -> f64 {
        self.0
    }
}

/// Text that has to say something: a reason, or the name of a target construct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyText;

/// Why a mapping is unsupported. Never blank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason(pub(crate) String);

impl Reason {
    /// A reason, which must contain something other than whitespace.
    pub fn new(text: &str) -> Result<Reason, EmptyText> {
        if text.trim().is_empty() { Err(EmptyText) } else { Ok(Reason(text.to_string())) }
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What an export did to one entity. See `docs/FIDELITY_TAXONOMY.md` for what each value claims and
/// the evidence it needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Fidelity {
    /// The same construct with the same parameters.
    Exact,
    /// The same locus on the same construct type, with parameters related by a documented exact map.
    ExactReparameterized,
    /// The same locus written as a different structure, for example arcs and lines for a conic.
    EquivalentCompound,
    /// A display approximation within a stated bound. Not a mathematical claim.
    BoundedApproximation(ErrorBound),
    /// Not translated, with the reason.
    Unsupported(Reason),
}

/// The five values without their payloads: what a report counts by and what the serialized form names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FidelityKind {
    /// See [`Fidelity::Exact`].
    Exact,
    /// See [`Fidelity::ExactReparameterized`].
    ExactReparameterized,
    /// See [`Fidelity::EquivalentCompound`].
    EquivalentCompound,
    /// See [`Fidelity::BoundedApproximation`].
    BoundedApproximation,
    /// See [`Fidelity::Unsupported`].
    Unsupported,
}

/// Which of the epic's three conformance classes a value is allowed to claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claims {
    /// Structural exactness: equivalent parameters where directly comparable.
    pub structural: bool,
    /// Geometric exactness: an equivalent locus under the numeric policy.
    pub geometric: bool,
    /// Display fidelity: a bounded rendering approximation.
    pub display: bool,
}

impl FidelityKind {
    /// Every value, in taxonomy order.
    pub const ALL: [FidelityKind; 5] = [
        FidelityKind::Exact,
        FidelityKind::ExactReparameterized,
        FidelityKind::EquivalentCompound,
        FidelityKind::BoundedApproximation,
        FidelityKind::Unsupported,
    ];

    /// The stable name, which is also what the serialized form writes.
    pub fn name(self) -> &'static str {
        match self {
            FidelityKind::Exact => "Exact",
            FidelityKind::ExactReparameterized => "ExactReparameterized",
            FidelityKind::EquivalentCompound => "EquivalentCompound",
            FidelityKind::BoundedApproximation => "BoundedApproximation",
            FidelityKind::Unsupported => "Unsupported",
        }
    }

    /// The value with this exact name.
    pub fn parse(name: &str) -> Option<FidelityKind> {
        FidelityKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The conformance classes this value may claim.
    pub fn claims(self) -> Claims {
        let (structural, geometric, display) = match self {
            FidelityKind::Exact | FidelityKind::ExactReparameterized => (true, true, false),
            FidelityKind::EquivalentCompound => (false, true, false),
            FidelityKind::BoundedApproximation => (false, false, true),
            FidelityKind::Unsupported => (false, false, false),
        };
        Claims { structural, geometric, display }
    }
}

impl Fidelity {
    /// The value without its payload.
    pub fn kind(&self) -> FidelityKind {
        match self {
            Fidelity::Exact => FidelityKind::Exact,
            Fidelity::ExactReparameterized => FidelityKind::ExactReparameterized,
            Fidelity::EquivalentCompound => FidelityKind::EquivalentCompound,
            Fidelity::BoundedApproximation(_) => FidelityKind::BoundedApproximation,
            Fidelity::Unsupported(_) => FidelityKind::Unsupported,
        }
    }
}

/// Whether the output construct is a native one or a tessellation (a polyline standing in for a curve).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation {
    /// The construct is the geometry itself.
    Native,
    /// The construct samples the geometry, so it can only ever be an approximation.
    Tessellated,
}

impl Representation {
    fn name(self) -> &'static str {
        match self {
            Representation::Native => "Native",
            Representation::Tessellated => "Tessellated",
        }
    }
}

/// The construct an export wrote, and whether it is native or tessellated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetConstruct {
    name: String,
    representation: Representation,
}

impl TargetConstruct {
    /// A target construct. The name must contain something other than whitespace.
    pub fn new(name: &str, representation: Representation) -> Result<TargetConstruct, EmptyText> {
        if name.trim().is_empty() {
            Err(EmptyText)
        } else {
            Ok(TargetConstruct { name: name.to_string(), representation })
        }
    }

    /// The construct's name in the target format.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Native or tessellated.
    pub fn representation(&self) -> Representation {
        self.representation
    }
}

/// Why a record was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// The source capability id is zero, which no capability uses.
    NoCapability,
    /// A tessellated output was recorded as a geometric exactness claim.
    TessellatedClaimedExact(FidelityKind),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordError::NoCapability => write!(f, "a record must name a source capability"),
            RecordError::TessellatedClaimedExact(k) => {
                write!(f, "a tessellated output cannot be recorded as {}", k.name())
            }
        }
    }
}

/// What one export did to one entity.
///
/// The fields are private and the only constructor enforces the rules, so a record that names no
/// capability or no target, or that records a tessellation as exact, does not exist.
#[derive(Clone, Debug, PartialEq)]
pub struct FidelityRecord {
    entity_id: EntityId,
    source_capability: CapabilityId,
    target_construct: TargetConstruct,
    fidelity: Fidelity,
    note: String,
}

impl FidelityRecord {
    /// A record, or the rule it breaks.
    pub fn new(
        entity_id: EntityId,
        source_capability: CapabilityId,
        target_construct: TargetConstruct,
        fidelity: Fidelity,
        note: &str,
    ) -> Result<FidelityRecord, RecordError> {
        if source_capability.get() == 0 {
            return Err(RecordError::NoCapability);
        }
        let kind = fidelity.kind();
        let exactness = matches!(
            kind,
            FidelityKind::Exact | FidelityKind::ExactReparameterized | FidelityKind::EquivalentCompound
        );
        if target_construct.representation == Representation::Tessellated && exactness {
            return Err(RecordError::TessellatedClaimedExact(kind));
        }
        Ok(FidelityRecord {
            entity_id,
            source_capability,
            target_construct,
            fidelity,
            note: note.to_string(),
        })
    }

    /// The entity.
    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    /// The capability the source geometry used.
    pub fn source_capability(&self) -> CapabilityId {
        self.source_capability
    }

    /// What the export wrote.
    pub fn target_construct(&self) -> &TargetConstruct {
        &self.target_construct
    }

    /// What the export claims about it.
    pub fn fidelity(&self) -> &Fidelity {
        &self.fidelity
    }

    /// The error bound, present exactly when the value is a bounded approximation.
    pub fn error_bound(&self) -> Option<ErrorBound> {
        match &self.fidelity {
            Fidelity::BoundedApproximation(b) => Some(*b),
            _ => None,
        }
    }

    /// Free text for a human. Never interpreted.
    pub fn note(&self) -> &str {
        &self.note
    }
}

/// The records an export accumulates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FidelityLedger {
    records: Vec<FidelityRecord>,
}

/// Why a serialized ledger was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The first line is not `pvec-fidelity-ledger MAJOR.MINOR`.
    BadHeader,
    /// The ledger was written by a build whose format this one cannot read.
    UnsupportedMajor(u32),
    /// The ledger ends before its trailer, or the trailer's count disagrees with the records.
    Truncated,
    /// Record number `record` (from 0) lacks a required field.
    MissingField {
        /// Index of the record.
        record: usize,
        /// The field name.
        field: &'static str,
    },
    /// A line, counted from 1, is not valid.
    Invalid {
        /// The line number.
        line: usize,
        /// What is wrong with it.
        what: &'static str,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::BadHeader => write!(f, "not a fidelity ledger"),
            ParseError::UnsupportedMajor(m) => write!(f, "ledger format major version {m} is not supported"),
            ParseError::Truncated => write!(f, "the ledger is truncated"),
            ParseError::MissingField { record, field } => write!(f, "record {record} has no {field}"),
            ParseError::Invalid { line, what } => write!(f, "line {line}: {what}"),
        }
    }
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

fn unescape(s: &str) -> Option<String> {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next()? {
                '\\' => o.push('\\'),
                'n' => o.push('\n'),
                'r' => o.push('\r'),
                _ => return None,
            }
        } else {
            o.push(c);
        }
    }
    Some(o)
}

fn parse_bits(s: &str) -> Option<f64> {
    let hex = s.strip_prefix("0x")?;
    if hex.len() != 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(f64::from_bits(u64::from_str_radix(hex, 16).ok()?))
}

#[derive(Default)]
struct Draft {
    entity: Option<u32>,
    capability: Option<u32>,
    representation: Option<Representation>,
    target: Option<String>,
    fidelity: Option<FidelityKind>,
    bound: Option<(usize, f64)>,
    reason: Option<String>,
    note: String,
}

impl FidelityLedger {
    /// An empty ledger.
    pub fn new() -> FidelityLedger {
        FidelityLedger::default()
    }

    /// Appends a record. Records keep the order they were added in.
    pub fn record(&mut self, record: FidelityRecord) {
        self.records.push(record);
    }

    /// The records, in order.
    pub fn records(&self) -> &[FidelityRecord] {
        &self.records
    }

    /// How many records carry this value.
    pub fn count(&self, kind: FidelityKind) -> usize {
        self.records.iter().filter(|r| r.fidelity.kind() == kind).count()
    }

    /// The source capability ids the registry does not know, each listed once, in order of first use.
    pub fn unregistered(&self, registry: &Registry) -> Vec<CapabilityId> {
        let mut out: Vec<CapabilityId> = Vec::new();
        for r in &self.records {
            let id = r.source_capability;
            if registry.get(id).is_none() && !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    /// The versioned text form. Deterministic: the same ledger always gives the same bytes.
    pub fn to_text(&self) -> String {
        let mut o = format!("{MAGIC} {FORMAT_MAJOR}.{FORMAT_MINOR}\n");
        for r in &self.records {
            o.push_str("record\n");
            o.push_str(&format!("entity {}\n", r.entity_id.index()));
            o.push_str(&format!("capability {}\n", r.source_capability.get()));
            o.push_str(&format!(
                "target {} {}\n",
                r.target_construct.representation.name(),
                escape(&r.target_construct.name)
            ));
            o.push_str(&format!("fidelity {}\n", r.fidelity.kind().name()));
            match &r.fidelity {
                Fidelity::BoundedApproximation(b) => {
                    o.push_str(&format!("error_bound 0x{:016x}\n", b.get().to_bits()));
                }
                Fidelity::Unsupported(reason) => o.push_str(&format!("reason {}\n", escape(reason.as_str()))),
                _ => {}
            }
            o.push_str("note");
            if !r.note.is_empty() {
                o.push(' ');
                o.push_str(&escape(&r.note));
            }
            o.push('\n');
            o.push_str("end\n");
        }
        o.push_str(&format!("ledger-end {}\n", self.records.len()));
        o
    }

    /// Reads the text form. Fields it does not know are skipped when the major version matches, and
    /// every rule of [`FidelityRecord::new`] applies again, so a forged ledger cannot smuggle in a
    /// record the constructor would have refused.
    pub fn from_text(text: &str) -> Result<FidelityLedger, ParseError> {
        if !text.is_empty() && !text.ends_with('\n') {
            return Err(ParseError::Truncated);
        }
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));
        let (_, header) = lines.next().ok_or(ParseError::BadHeader)?;
        let version =
            header.strip_prefix(MAGIC).and_then(|r| r.strip_prefix(' ')).ok_or(ParseError::BadHeader)?;
        let (major, minor) = version.split_once('.').ok_or(ParseError::BadHeader)?;
        let major: u32 = major.parse().map_err(|_| ParseError::BadHeader)?;
        let _minor: u32 = minor.parse().map_err(|_| ParseError::BadHeader)?;
        if major != FORMAT_MAJOR {
            return Err(ParseError::UnsupportedMajor(major));
        }

        let mut ledger = FidelityLedger::new();
        let mut draft: Option<Draft> = None;
        for (n, line) in lines {
            let (key, value) = match line.split_once(' ') {
                Some((k, v)) => (k, v),
                None => (line, ""),
            };
            let Some(d) = draft.as_mut() else {
                match key {
                    "record" => draft = Some(Draft::default()),
                    "ledger-end" => {
                        let count: usize = value
                            .parse()
                            .map_err(|_| ParseError::Invalid { line: n, what: "bad record count" })?;
                        return if count == ledger.records.len() {
                            Ok(ledger)
                        } else {
                            Err(ParseError::Truncated)
                        };
                    }
                    _ => {} // a field of a later minor version
                }
                continue;
            };
            let bad = |what| ParseError::Invalid { line: n, what };
            match key {
                "end" => {
                    let index = ledger.records.len();
                    let d = draft.take().unwrap_or_default();
                    ledger.records.push(finish(d, index, n)?);
                }
                "entity" => d.entity = Some(value.parse().map_err(|_| bad("bad entity id"))?),
                "capability" => d.capability = Some(value.parse().map_err(|_| bad("bad capability id"))?),
                "target" => {
                    let (rep, name) = value.split_once(' ').ok_or(bad("bad target"))?;
                    d.representation = Some(match rep {
                        "Native" => Representation::Native,
                        "Tessellated" => Representation::Tessellated,
                        _ => return Err(bad("unknown representation")),
                    });
                    d.target = Some(unescape(name).ok_or(bad("bad escape"))?);
                }
                "fidelity" => {
                    d.fidelity = Some(FidelityKind::parse(value).ok_or(bad("unknown fidelity value"))?)
                }
                "error_bound" => d.bound = Some((n, parse_bits(value).ok_or(bad("bad error bound"))?)),
                "reason" => d.reason = Some(unescape(value).ok_or(bad("bad escape"))?),
                "note" => d.note = unescape(value).ok_or(bad("bad escape"))?,
                _ => {} // a field of a later minor version
            }
        }
        Err(ParseError::Truncated)
    }
}

fn finish(d: Draft, record: usize, end_line: usize) -> Result<FidelityRecord, ParseError> {
    let invalid = |what| ParseError::Invalid { line: end_line, what };
    let missing = |field| ParseError::MissingField { record, field };
    let entity =
        EntityId::new(d.entity.ok_or(missing("entity"))?).map_err(|_| invalid("entity id out of range"))?;
    let capability = d.capability.ok_or(missing("capability"))?;
    let name = d.target.ok_or(missing("target"))?;
    let representation = d.representation.ok_or(missing("target"))?;
    let kind = d.fidelity.ok_or(missing("fidelity"))?;
    let target =
        TargetConstruct::new(&name, representation).map_err(|_| invalid("blank target construct"))?;
    let fidelity = match kind {
        FidelityKind::BoundedApproximation => {
            let (_, v) = d.bound.ok_or(missing("error_bound"))?;
            Fidelity::BoundedApproximation(
                ErrorBound::new(v).map_err(|_| invalid("error bound is not finite and positive"))?,
            )
        }
        _ if d.bound.is_some() => return Err(invalid("only a bounded approximation carries an error bound")),
        FidelityKind::Unsupported => {
            let r = d.reason.ok_or(missing("reason"))?;
            Fidelity::Unsupported(Reason::new(&r).map_err(|_| invalid("blank reason"))?)
        }
        FidelityKind::Exact => Fidelity::Exact,
        FidelityKind::ExactReparameterized => Fidelity::ExactReparameterized,
        FidelityKind::EquivalentCompound => Fidelity::EquivalentCompound,
    };
    FidelityRecord::new(entity, CapabilityId::new(capability), target, fidelity, &d.note)
        .map_err(|_| invalid("the record breaks a fidelity rule"))
}
