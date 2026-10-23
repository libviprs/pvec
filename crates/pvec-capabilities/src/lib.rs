//! The PVec capability registry (issue P3.4).
//!
//! Readers and writers need one authority on what a capability is, what it requires, and how an
//! older reader must behave when it meets a newer file. This crate is that authority, as data: one
//! [`Capability`] per entry with a stable numeric id, a semantic version, the ISO 10303-42 construct
//! it is aligned to, a required-or-optional status, its STEP and GIS behaviour and the minimum reader
//! version. `docs/CAPABILITIES.md` is generated from the same table by [`render_markdown`], and a
//! repository check fails the gate if the committed file differs from the generated one by a byte.
//!
//! The registry is not a trait per capability. Geometry codecs and component streams read their ids
//! and versions from [`ids`] and [`Registry::v1`].
//!
//! # What the STEP and GIS columns mean
//!
//! They state what the semantic profile declares, not what a round trip has demonstrated. Whether a
//! declared exact form really comes back to the same locus is measured in phases 2 and 6.
//!
//! The crate has no dependencies, no I/O and no `unsafe`.

use core::fmt;

mod extension;

pub use extension::{
    EvidenceClass, ExtensionError, ExtensionFixture, MissingArtifact, ProposalProblem, ProposalSummary,
    check_proposal,
};

/// A stable numeric capability id. Ids are never reused: a removed capability moves to the retired
/// list and its number stays taken for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityId(u32);

impl CapabilityId {
    /// Wraps a number. Whether the id is registered is the registry's business.
    pub const fn new(id: u32) -> CapabilityId {
        CapabilityId(id)
    }

    /// The number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The ids of the v1 capabilities. The numbers of the geometry capabilities are the section numbers
/// of `docs/ISO_PROFILE.md`, and a test pins every one of them.
pub mod ids {
    use super::CapabilityId;

    /// Section 1.
    pub const CARTESIAN_POINT: CapabilityId = CapabilityId::new(1);
    /// Section 2.
    pub const DIRECTION_VECTOR: CapabilityId = CapabilityId::new(2);
    /// Section 3.
    pub const AXIS_PLACEMENT_2D: CapabilityId = CapabilityId::new(3);
    /// Section 4.
    pub const LINE: CapabilityId = CapabilityId::new(4);
    /// Section 5.
    pub const POLYLINE: CapabilityId = CapabilityId::new(5);
    /// Section 6.
    pub const CIRCLE: CapabilityId = CapabilityId::new(6);
    /// Section 7.
    pub const ELLIPSE: CapabilityId = CapabilityId::new(7);
    /// Section 8.
    pub const TRIMMED_CURVE: CapabilityId = CapabilityId::new(8);
    /// Section 9.
    pub const COMPOSITE_CURVE: CapabilityId = CapabilityId::new(9);
    /// Section 10.
    pub const BEZIER_CURVE: CapabilityId = CapabilityId::new(10);
    /// Section 11.
    pub const B_SPLINE_CURVE: CapabilityId = CapabilityId::new(11);
    /// Section 12.
    pub const RATIONAL_B_SPLINE_CURVE: CapabilityId = CapabilityId::new(12);
    /// Section 13.
    pub const AFFINE_TRANSFORM_2D: CapabilityId = CapabilityId::new(13);
    /// The layer stream (P3.7).
    pub const LAYER: CapabilityId = CapabilityId::new(14);
    /// The line style stream (P3.7).
    pub const LINE_STYLE: CapabilityId = CapabilityId::new(15);
    /// The text stream (P3.7).
    pub const TEXT: CapabilityId = CapabilityId::new(16);
    /// The hatch stream (P3.7).
    pub const HATCH: CapabilityId = CapabilityId::new(17);
    /// The source provenance stream (P3.7).
    pub const SOURCE_PROVENANCE: CapabilityId = CapabilityId::new(18);
    /// The reserved space for future DWG semantics (P3.7).
    pub const DWG_SEMANTICS: CapabilityId = CapabilityId::new(19);
    /// The semantic class stream (P3.9).
    pub const SEMANTIC_CLASS: CapabilityId = CapabilityId::new(20);
}

/// A semantic version. Ordering is major, then minor, then patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Version {
    /// Incompatible change.
    pub major: u16,
    /// Compatible addition.
    pub minor: u16,
    /// Compatible fix.
    pub patch: u16,
}

impl Version {
    /// A version.
    pub const fn new(major: u16, minor: u16, patch: u16) -> Version {
        Version { major, minor, patch }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What kind of capability an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Class {
    /// A curve, point or transform of the semantic profile. Aligned to a named ISO construct.
    Geometry,
    /// An independent stream (layer, style, text, hatch, provenance). Outside the geometry oracle, so
    /// it names no ISO construct and is always optional.
    Component,
    /// A custom geometry extension that earned its place through `docs/EXTENSIONS.md`. It names no ISO
    /// construct (it exists because none fits), is optional unless an ADR is listed, and is registered only
    /// through [`Registry::register_extension`].
    Extension,
}

/// Whether a reader that lacks the capability may still open the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// A reader without it refuses the whole file; nothing is rendered partially.
    Required,
    /// A reader without it skips the capability's data.
    Optional,
}

/// What the profile declares for the STEP reference export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StepBehaviour {
    /// The profile names an exact Part 21 form.
    ExactForm,
    /// There is no exact form; the export is a bounded approximation with its error reported.
    BoundedApproximation,
    /// Not part of a STEP export.
    NotApplicable,
}

/// What the profile declares for the GIS exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GisBehaviour {
    /// Representable exactly in the simple-feature types the exports use.
    Exact,
    /// Densified into line work within a reported error budget, a display-fidelity result.
    Densified,
    /// Not part of a GIS export.
    NotApplicable,
}

/// One registry entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Capability {
    /// The stable id.
    pub id: CapabilityId,
    /// The stable machine name.
    pub name: &'static str,
    /// Geometry or component.
    pub class: Class,
    /// The semantic version of this capability.
    pub version: Version,
    /// The ISO 10303-42 construct names this entry is aligned to, comma separated. Empty for a
    /// component.
    pub iso_construct: &'static str,
    /// The number of the section of `docs/ISO_PROFILE.md` that defines it. Zero for a component.
    pub profile_section: u32,
    /// Required or optional.
    pub requirement: Requirement,
    /// The STEP behaviour the profile declares.
    pub step: StepBehaviour,
    /// The GIS behaviour the profile declares.
    pub gis: GisBehaviour,
    /// The lowest reader version that understands this capability.
    pub min_reader: Version,
    /// The restriction PVec places on the construct, in a line.
    pub restriction: &'static str,
    /// The ADR that justifies marking an extension required. Always `None` for the v1 geometry and
    /// component entries.
    pub adr: Option<&'static str>,
}

/// An id that was assigned once and must never be assigned again.
#[derive(Clone, Debug, PartialEq)]
pub struct Retired {
    /// The id.
    pub id: CapabilityId,
    /// What it was called.
    pub name: &'static str,
    /// Why it went away.
    pub reason: &'static str,
}

/// One numbered section of `docs/ISO_PROFILE.md`, as far as validation needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSection {
    /// The section number.
    pub number: u32,
    /// The text of the section's "ISO 10303-42:2025 construct" field.
    pub constructs: String,
}

/// A registry that failed validation, naming the entry and the rule.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistryError {
    /// Id zero is reserved as "no capability".
    ZeroId,
    /// Two live entries share an id.
    DuplicateId(CapabilityId),
    /// Two live entries share a name.
    DuplicateName(&'static str),
    /// The retired list names an id twice.
    DuplicateRetired(CapabilityId),
    /// A live entry uses an id on the retired list.
    ReusedRetiredId(CapabilityId),
    /// An id below the highest one is neither live nor retired: it was deleted and could be reused.
    UnaccountedId(CapabilityId),
    /// A component is registered as required.
    ComponentMustBeOptional(&'static str),
    /// A geometry entry names no ISO construct.
    NoConstruct(&'static str),
    /// The profile document has no section with the entry's number.
    NoProfileSection {
        /// The entry.
        name: &'static str,
        /// The section number it points at.
        section: u32,
    },
    /// The section exists but does not name the construct.
    ConstructNotInSection {
        /// The entry.
        name: &'static str,
        /// The construct that was not found.
        construct: &'static str,
        /// The section.
        section: u32,
    },
    /// Two entries claim the same construct and the profile has no separate section for each.
    AmbiguousConstruct {
        /// The construct.
        construct: &'static str,
        /// The lower id of the two.
        first: &'static str,
        /// The other.
        second: &'static str,
    },
    /// A component entry names an ISO construct.
    ComponentNamesConstruct(&'static str),
    /// An extension entry names an ISO construct. Extensions exist because no construct fits.
    ExtensionNamesConstruct(&'static str),
    /// An extension is registered as required without an ADR listed in its entry.
    ExtensionRequiredWithoutAdr(&'static str),
    /// No entry has this id.
    UnknownId(CapabilityId),
    /// The entry is not a component, so its version is not bumped this way.
    NotAComponent(CapabilityId),
    /// A version bump that does not go up.
    VersionNotIncreased {
        /// The capability.
        id: CapabilityId,
        /// Its current version.
        current: Version,
        /// The version asked for.
        requested: Version,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::ZeroId => write!(f, "capability id 0 is reserved"),
            RegistryError::DuplicateId(id) => write!(f, "capability id {id} is used by two entries"),
            RegistryError::DuplicateName(n) => write!(f, "capability name {n} is used by two entries"),
            RegistryError::DuplicateRetired(id) => write!(f, "retired id {id} is listed twice"),
            RegistryError::ReusedRetiredId(id) => {
                write!(f, "capability id {id} is retired and cannot be reused")
            }
            RegistryError::UnaccountedId(id) => {
                write!(
                    f,
                    "capability id {id} is neither registered nor retired, so it could be handed out again"
                )
            }
            RegistryError::ComponentMustBeOptional(n) => write!(f, "component {n} must be optional"),
            RegistryError::NoConstruct(n) => {
                write!(f, "geometry capability {n} names no ISO 10303-42 construct")
            }
            RegistryError::NoProfileSection { name, section } => {
                write!(f, "capability {name} points at profile section {section}, which does not exist")
            }
            RegistryError::ConstructNotInSection { name, construct, section } => {
                write!(
                    f,
                    "capability {name}: profile section {section} does not name the construct {construct}"
                )
            }
            RegistryError::AmbiguousConstruct { construct, first, second } => write!(
                f,
                "capabilities {first} and {second} claim the construct {construct} but the profile does not distinguish them"
            ),
            RegistryError::ComponentNamesConstruct(n) => {
                write!(
                    f,
                    "component {n} must not name an ISO construct: components are outside the geometry oracle"
                )
            }
            RegistryError::ExtensionNamesConstruct(n) => {
                write!(f, "extension {n} must not name an ISO construct: it exists because none fits")
            }
            RegistryError::ExtensionRequiredWithoutAdr(n) => {
                write!(f, "extension {n} is required but lists no ADR; extensions are optional by default")
            }
            RegistryError::UnknownId(id) => write!(f, "no capability has id {id}"),
            RegistryError::NotAComponent(id) => write!(f, "capability {id} is not a component"),
            RegistryError::VersionNotIncreased { id, current, requested } => {
                write!(f, "capability {id} is at version {current}; {requested} is not higher")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// The reader side of a negotiation: its own version and the capabilities it implements.
#[derive(Clone, Debug, PartialEq)]
pub struct Reader {
    /// The reader's version.
    pub version: Version,
    /// The capabilities it implements.
    pub implemented: Vec<CapabilityId>,
}

/// One capability a file declares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Declared {
    /// Which capability.
    pub id: CapabilityId,
    /// Whether the file says a reader must have it.
    pub requirement: Requirement,
    /// The version the file was written with.
    pub version: Version,
}

/// What a file declares up front.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// The lowest reader version the file may be opened with.
    pub min_reader: Version,
    /// The capabilities it uses.
    pub capabilities: Vec<Declared>,
}

/// The outcome of an accepted negotiation.
#[derive(Clone, Debug, PartialEq)]
pub struct Negotiated {
    /// Optional capabilities the reader does not implement and will skip.
    pub skipped: Vec<CapabilityId>,
}

/// Why a file is refused. A refusal carries no partial result: the file is not rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NegotiationError {
    /// The file needs a newer reader.
    ReaderTooOld {
        /// What the file (or a capability it declares) requires.
        file_requires: Version,
        /// What the reader is.
        reader: Version,
    },
    /// A required capability is not implemented, or not even known.
    MissingRequired {
        /// Which.
        id: CapabilityId,
        /// Its name, if the registry knows the id.
        name: Option<&'static str>,
    },
    /// The file marked a component as required, which components never are.
    ComponentDeclaredRequired(CapabilityId),
    /// The file marked an extension as required while the registry lists it as optional (no ADR).
    ExtensionDeclaredRequired(CapabilityId),
}

impl fmt::Display for NegotiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NegotiationError::ReaderTooOld { file_requires, reader } => {
                write!(
                    f,
                    "the file needs reader version {file_requires} or newer and this reader is {reader}"
                )
            }
            NegotiationError::MissingRequired { id, name: Some(n) } => {
                write!(f, "required capability {n} (id {id}) is not implemented by this reader")
            }
            NegotiationError::MissingRequired { id, name: None } => {
                write!(f, "required capability with unknown id {id} is not implemented by this reader")
            }
            NegotiationError::ComponentDeclaredRequired(id) => {
                write!(f, "capability id {id} is a component and cannot be declared required")
            }
            NegotiationError::ExtensionDeclaredRequired(id) => {
                write!(
                    f,
                    "capability id {id} is an optional extension and cannot be declared required by a file"
                )
            }
        }
    }
}

impl std::error::Error for NegotiationError {}

/// The registry: entries and the retired ids, validated on construction.
#[derive(Clone, Debug, PartialEq)]
pub struct Registry {
    entries: Vec<Capability>,
    retired: Vec<Retired>,
}

impl Registry {
    /// Builds a registry and checks the rules that need no other document: ids are non-zero, unique
    /// and never reused from the retired list; every id up to the highest is live or retired, so a
    /// deleted entry cannot be forgotten; names are unique; components are optional.
    pub fn new(mut entries: Vec<Capability>, retired: Vec<Retired>) -> Result<Registry, RegistryError> {
        entries.sort_by_key(|c| c.id);
        let mut retired = retired;
        retired.sort_by_key(|r| r.id);
        if entries.iter().any(|c| c.id.get() == 0) || retired.iter().any(|r| r.id.get() == 0) {
            return Err(RegistryError::ZeroId);
        }
        for w in entries.windows(2) {
            if w[0].id == w[1].id {
                return Err(RegistryError::DuplicateId(w[0].id));
            }
        }
        let mut names: Vec<&'static str> = entries.iter().map(|c| c.name).collect();
        names.sort_unstable();
        for w in names.windows(2) {
            if w[0] == w[1] {
                return Err(RegistryError::DuplicateName(w[0]));
            }
        }
        for w in retired.windows(2) {
            if w[0].id == w[1].id {
                return Err(RegistryError::DuplicateRetired(w[0].id));
            }
        }
        if let Some(c) = entries.iter().find(|c| retired.iter().any(|r| r.id == c.id)) {
            return Err(RegistryError::ReusedRetiredId(c.id));
        }
        // Every id from 1 to the highest must be live or retired. Merge the two sorted lists and look
        // for the first number that is missing; the work is proportional to the number of entries.
        let mut all: Vec<u32> =
            entries.iter().map(|c| c.id.get()).chain(retired.iter().map(|r| r.id.get())).collect();
        all.sort_unstable();
        for (i, id) in all.iter().enumerate() {
            let expected = i as u32 + 1;
            if *id != expected {
                return Err(RegistryError::UnaccountedId(CapabilityId::new(expected)));
            }
        }
        if let Some(c) =
            entries.iter().find(|c| c.class == Class::Component && c.requirement == Requirement::Required)
        {
            return Err(RegistryError::ComponentMustBeOptional(c.name));
        }
        if let Some(c) = entries.iter().find(|c| {
            c.class == Class::Extension
                && c.requirement == Requirement::Required
                && c.adr.is_none_or(|a| a.is_empty())
        }) {
            return Err(RegistryError::ExtensionRequiredWithoutAdr(c.name));
        }
        Ok(Registry { entries, retired })
    }

    /// The entries, in id order.
    pub fn entries(&self) -> &[Capability] {
        &self.entries
    }

    /// The retired ids, in id order.
    pub fn retired(&self) -> &[Retired] {
        &self.retired
    }

    /// The entry with this id.
    pub fn get(&self, id: CapabilityId) -> Option<&Capability> {
        self.entries.binary_search_by_key(&id, |c| c.id).ok().map(|i| &self.entries[i])
    }

    /// The entry with this name.
    pub fn by_name(&self, name: &str) -> Option<&Capability> {
        self.entries.iter().find(|c| c.name == name)
    }

    /// True when the id was assigned once and taken out of service.
    pub fn is_retired(&self, id: CapabilityId) -> bool {
        self.retired.binary_search_by_key(&id, |r| r.id).is_ok()
    }

    /// Checks the coupling to `docs/ISO_PROFILE.md`: every geometry entry names a construct, points
    /// at a section that exists and names that construct, and two entries on one construct are
    /// distinguished by separate sections. A component names no construct.
    pub fn validate_against_profile(&self, profile: &[ProfileSection]) -> Result<(), RegistryError> {
        for c in &self.entries {
            match c.class {
                Class::Component => {
                    if !c.iso_construct.is_empty() {
                        return Err(RegistryError::ComponentNamesConstruct(c.name));
                    }
                }
                Class::Extension => {
                    if !c.iso_construct.is_empty() {
                        return Err(RegistryError::ExtensionNamesConstruct(c.name));
                    }
                }
                Class::Geometry => {
                    if c.iso_construct.trim().is_empty() {
                        return Err(RegistryError::NoConstruct(c.name));
                    }
                    let Some(s) = profile.iter().find(|s| s.number == c.profile_section) else {
                        return Err(RegistryError::NoProfileSection {
                            name: c.name,
                            section: c.profile_section,
                        });
                    };
                    let tokens = identifiers(&s.constructs);
                    for construct in c.iso_construct.split(',').map(str::trim) {
                        if !tokens.iter().any(|t| t == construct) {
                            return Err(RegistryError::ConstructNotInSection {
                                name: c.name,
                                construct,
                                section: c.profile_section,
                            });
                        }
                    }
                }
            }
        }
        let geometry: Vec<&Capability> = self.entries.iter().filter(|c| c.class == Class::Geometry).collect();
        for (i, a) in geometry.iter().enumerate() {
            for b in &geometry[i + 1..] {
                if a.iso_construct == b.iso_construct && a.profile_section == b.profile_section {
                    return Err(RegistryError::AmbiguousConstruct {
                        construct: a.iso_construct,
                        first: a.name,
                        second: b.name,
                    });
                }
            }
        }
        Ok(())
    }

    /// Decides whether `reader` may open a file with this `file` declaration.
    ///
    /// The reader version is checked first. A required capability the reader lacks refuses the file
    /// outright. A file cannot weaken a registered requirement by declaring it optional, and cannot
    /// declare a component required. An unknown optional capability is skipped, an unknown required
    /// one refuses. Nothing is rendered partially: the result is a refusal or an acceptance.
    pub fn negotiate(&self, reader: &Reader, file: &Declaration) -> Result<Negotiated, NegotiationError> {
        if file.min_reader > reader.version {
            return Err(NegotiationError::ReaderTooOld {
                file_requires: file.min_reader,
                reader: reader.version,
            });
        }
        let mut skipped = Vec::new();
        for d in &file.capabilities {
            let has = reader.implemented.contains(&d.id);
            match self.get(d.id) {
                Some(c) => {
                    if c.min_reader > reader.version {
                        return Err(NegotiationError::ReaderTooOld {
                            file_requires: c.min_reader,
                            reader: reader.version,
                        });
                    }
                    if c.class == Class::Component && d.requirement == Requirement::Required {
                        return Err(NegotiationError::ComponentDeclaredRequired(d.id));
                    }
                    if c.class == Class::Extension
                        && c.requirement == Requirement::Optional
                        && d.requirement == Requirement::Required
                    {
                        return Err(NegotiationError::ExtensionDeclaredRequired(d.id));
                    }
                    let required =
                        c.requirement == Requirement::Required || d.requirement == Requirement::Required;
                    if !has {
                        if required {
                            return Err(NegotiationError::MissingRequired { id: d.id, name: Some(c.name) });
                        }
                        skipped.push(d.id);
                    }
                }
                None => {
                    if !has {
                        if d.requirement == Requirement::Required {
                            return Err(NegotiationError::MissingRequired { id: d.id, name: None });
                        }
                        skipped.push(d.id);
                    }
                }
            }
        }
        Ok(Negotiated { skipped })
    }

    /// Raises the version of a component stream. Geometry versions are not touched by this, and a
    /// component version never moves backwards or stays put, so a bump is always visible.
    pub fn bump_component_version(&mut self, id: CapabilityId, to: Version) -> Result<(), RegistryError> {
        let Some(i) = self.entries.iter().position(|c| c.id == id) else {
            return Err(RegistryError::UnknownId(id));
        };
        let c = &mut self.entries[i];
        if c.class != Class::Component {
            return Err(RegistryError::NotAComponent(id));
        }
        if to <= c.version {
            return Err(RegistryError::VersionNotIncreased { id, current: c.version, requested: to });
        }
        c.version = to;
        Ok(())
    }

    /// The v1 registry: the thirteen capabilities of the semantic profile and the seven component streams.
    pub fn v1() -> Registry {
        let g = |id: CapabilityId,
                 name: &'static str,
                 iso_construct: &'static str,
                 step: StepBehaviour,
                 gis: GisBehaviour,
                 restriction: &'static str| Capability {
            id,
            name,
            class: Class::Geometry,
            version: Version::new(1, 0, 0),
            iso_construct,
            profile_section: id.get(),
            requirement: Requirement::Required,
            step,
            gis,
            min_reader: Version::new(1, 0, 0),
            restriction,
            adr: None,
        };
        // A component stream: optional by construction, no ISO construct, no STEP or GIS behaviour.
        let c = |id: CapabilityId, name: &'static str, restriction: &'static str| Capability {
            id,
            name,
            class: Class::Component,
            version: Version::new(1, 0, 0),
            iso_construct: "",
            profile_section: 0,
            requirement: Requirement::Optional,
            step: StepBehaviour::NotApplicable,
            gis: GisBehaviour::NotApplicable,
            min_reader: Version::new(1, 0, 0),
            restriction,
            adr: None,
        };
        use GisBehaviour as G;
        use StepBehaviour as S;
        let entries = vec![
            g(
                ids::CARTESIAN_POINT,
                "cartesian_point",
                "cartesian_point",
                S::ExactForm,
                G::Exact,
                "exactly two finite f64 coordinates; negative zero is stored as positive zero",
            ),
            g(
                ids::DIRECTION_VECTOR,
                "direction_vector",
                "direction, vector",
                S::ExactForm,
                G::NotApplicable,
                "a unit direction, or a direction plus a strictly positive magnitude",
            ),
            g(
                ids::AXIS_PLACEMENT_2D,
                "axis_placement_2d",
                "axis2_placement_2d",
                S::ExactForm,
                G::NotApplicable,
                "a location and a unit reference direction; always right-handed",
            ),
            g(
                ids::LINE,
                "line",
                "line",
                S::ExactForm,
                G::NotApplicable,
                "unit direction, so the parameter is arc length; bounded only through a trimmed curve",
            ),
            g(
                ids::POLYLINE,
                "polyline",
                "polyline",
                S::ExactForm,
                G::Exact,
                "2 to 1048576 vertices, vertex i at t = i, no zero-length segment",
            ),
            g(
                ids::CIRCLE,
                "circle",
                "circle",
                S::ExactForm,
                G::Densified,
                "radius strictly positive; canonical domain from 0 to 2 pi",
            ),
            g(
                ids::ELLIPSE,
                "ellipse",
                "ellipse",
                S::ExactForm,
                G::Densified,
                "semi-axes are not reordered; axis ratio at most one million",
            ),
            g(
                ids::TRIMMED_CURVE,
                "trimmed_curve",
                "trimmed_curve",
                S::ExactForm,
                G::Densified,
                "basis is a line, circle, ellipse, Bezier or B-spline; parameter trims only; exact in GIS only over a line",
            ),
            g(
                ids::COMPOSITE_CURVE,
                "composite_curve",
                "composite_curve",
                S::ExactForm,
                G::Densified,
                "segments join end to start; at most 65536 segments; exact in GIS only when every segment is straight",
            ),
            g(
                ids::BEZIER_CURVE,
                "bezier_curve",
                "bezier_curve",
                S::ExactForm,
                G::Densified,
                "degree 1 to 16, non-rational, domain 0 to 1",
            ),
            g(
                ids::B_SPLINE_CURVE,
                "b_spline_curve",
                "b_spline_curve_with_knots",
                S::ExactForm,
                G::Densified,
                "degree 1 to 16, explicit knots, non-rational, no wrapped periodic form",
            ),
            g(
                ids::RATIONAL_B_SPLINE_CURVE,
                "rational_b_spline_curve",
                "rational_b_spline_curve",
                S::ExactForm,
                G::Densified,
                "a B-spline plus strictly positive weights; equal weights stay rational",
            ),
            g(
                ids::AFFINE_TRANSFORM_2D,
                "affine_transform_2d",
                "cartesian_transformation_operator_2d",
                S::ExactForm,
                G::NotApplicable,
                "any invertible 2x3 affine map; only the similarity subset has a Part 21 operator",
            ),
            c(ids::LAYER, "layer", "a layer table and a layer index per entity, by EntityId"),
            c(
                ids::LINE_STYLE,
                "line_style",
                "a dash pattern table and a style index per entity, by EntityId",
            ),
            c(ids::TEXT, "text", "text records anchored at a point, each referring to an entity by EntityId"),
            c(ids::HATCH, "hatch", "boundary by EntityId references plus fill parameters; embeds no curve"),
            c(
                ids::SOURCE_PROVENANCE,
                "source_provenance",
                "source handle bytes per EntityId; never inside the id",
            ),
            c(
                ids::DWG_SEMANTICS,
                "dwg_semantics",
                "reserved space for future DWG semantics; opaque to a reader that does not know it",
            ),
            c(
                ids::SEMANTIC_CLASS,
                "semantic_class",
                "element classes (wall, column) by EntityId and room and corridor regions bounded by EntityId rings; embeds no curve",
            ),
        ];
        Registry::new(entries, Vec::new()).expect("the built-in registry is valid; a test pins it")
    }
}

/// The identifiers (letters, digits and underscores) in a piece of profile text.
fn identifiers(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Geometry => "geometry",
            Class::Component => "component",
            Class::Extension => "extension",
        }
    }
}

impl Requirement {
    fn label(self) -> &'static str {
        match self {
            Requirement::Required => "required",
            Requirement::Optional => "optional",
        }
    }
}

impl StepBehaviour {
    fn label(self) -> &'static str {
        match self {
            StepBehaviour::ExactForm => "exact form",
            StepBehaviour::BoundedApproximation => "bounded approximation, error reported",
            StepBehaviour::NotApplicable => "not applicable",
        }
    }
}

impl GisBehaviour {
    fn label(self) -> &'static str {
        match self {
            GisBehaviour::Exact => "exact",
            GisBehaviour::Densified => "densified, error reported",
            GisBehaviour::NotApplicable => "not applicable",
        }
    }
}

/// Renders `docs/CAPABILITIES.md` from a registry. The output is deterministic, ends in exactly one
/// newline, and a repository check compares it with the committed file byte for byte.
pub fn render_markdown(registry: &Registry) -> String {
    let mut out = String::new();
    out.push_str("# PVec capabilities\n\n");
    out.push_str(
        "This file is generated from the registry in `crates/pvec-capabilities`. I do not edit it by hand: \
         the gate regenerates it and fails if the committed file differs by a single byte. To change it I \
         change the registry and run `PVEC_BLESS_CAPABILITIES=1 cargo test -p pvec-checks --test \
         capabilities_doc`.\n\n",
    );
    out.push_str(
        "The STEP and GIS columns are what the semantic profile declares for each capability. Whether a \
         declared exact form really round-trips to the same locus is measured in phases 2 and 6, not here.\n\n",
    );
    out.push_str("## Registry\n\n");
    out.push_str(
        "| id | name | class | version | ISO 10303-42 construct | profile section | requirement | STEP | GIS | min reader | ADR | restriction |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for c in registry.entries() {
        let construct = if c.iso_construct.is_empty() {
            "none".to_string()
        } else {
            c.iso_construct.split(',').map(|s| format!("`{}`", s.trim())).collect::<Vec<_>>().join(", ")
        };
        let section = if c.profile_section == 0 { "none".to_string() } else { c.profile_section.to_string() };
        out.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            c.id,
            c.name,
            c.class.label(),
            c.version,
            construct,
            section,
            c.requirement.label(),
            c.step.label(),
            c.gis.label(),
            c.min_reader,
            c.adr.unwrap_or("none"),
            c.restriction
        ));
    }
    out.push_str("\n## Retired ids\n\n");
    if registry.retired().is_empty() {
        out.push_str("None yet. A retired id stays here for good and is never assigned again.\n");
    } else {
        out.push_str("| id | name | reason |\n|---|---|---|\n");
        for r in registry.retired() {
            out.push_str(&format!("| {} | `{}` | {} |\n", r.id, r.name, r.reason));
        }
    }
    out
}
