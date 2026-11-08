//! Capability negotiation and unknown-block skipping (issue P4.7).
//!
//! A reader that meets a block it has never heard of has to skip it correctly without parsing its
//! payload, or every future capability is a breaking change. This module is the match between what a file
//! declares (its manifest's capability list, from P3.4, and the descriptors of its blocks) and what a
//! build supports ([`ReaderCapabilities`]), and the three outcomes of it: [`Outcome::Open`],
//! [`Outcome::OpenDegraded`] with a report of what was skipped, and [`Outcome::Refuse`] naming the
//! capability and its version. Chapter 5 of `docs/CONTAINER.md` is the contract and is rendered from the
//! constants here.

use crate::directory::{BlockEntry, BlockKind};
use crate::manifest::Manifest;
use crate::reader::BlockId;
use pvec_capabilities::{CapabilityId, Class, Declared, Registry, Requirement, Version};
use std::fmt;
use std::sync::OnceLock;

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(Registry::v1)
}

/// The capabilities, and the versions of them, a build supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderCapabilities {
    /// The version of the capability registry this reader implements.
    pub version: Version,
    supported: Vec<(CapabilityId, u16, u16)>,
}

impl ReaderCapabilities {
    /// Every capability of the v1 registry, at the version the registry gives it.
    pub fn v1() -> ReaderCapabilities {
        let supported =
            registry().entries().iter().map(|c| (c.id, c.version.major, c.version.minor)).collect();
        ReaderCapabilities { version: pvec_capabilities::FORMAT_VERSION, supported }
    }

    /// A reader of registry version `version` that supports nothing yet.
    pub fn none(version: Version) -> ReaderCapabilities {
        ReaderCapabilities { version, supported: Vec::new() }
    }

    /// Supports capability `id` up to `major.minor`: files at that major and any minor opens, since a
    /// newer minor only adds.
    pub fn with(mut self, id: CapabilityId, major: u16, minor: u16) -> ReaderCapabilities {
        self.supported.retain(|(i, _, _)| *i != id);
        self.supported.push((id, major, minor));
        self
    }

    /// Stops supporting capability `id`.
    pub fn without(mut self, id: CapabilityId) -> ReaderCapabilities {
        self.supported.retain(|(i, _, _)| *i != id);
        self
    }

    /// The `(major, minor)` this reader supports of `id`.
    pub fn supports(&self, id: CapabilityId) -> Option<(u16, u16)> {
        self.supported.iter().find(|(i, _, _)| *i == id).map(|(_, a, b)| (*a, *b))
    }
}

impl Default for ReaderCapabilities {
    fn default() -> ReaderCapabilities {
        ReaderCapabilities::v1()
    }
}

/// Why a file is refused. The refusal names the capability and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The file needs a newer reader.
    ReaderTooOld {
        /// What the file requires.
        file_requires: Version,
        /// What the reader is.
        reader: Version,
    },
    /// A required capability is not supported, or not even known.
    MissingRequired {
        /// The capability.
        id: CapabilityId,
        /// The version the file declares.
        version: Version,
        /// Its name, when the registry knows the id.
        name: Option<&'static str>,
    },
    /// A required capability is declared at a major version the reader does not support.
    IncompatibleMajor {
        /// The capability.
        id: CapabilityId,
        /// The version the file declares.
        file: Version,
        /// The major version the reader supports.
        supported_major: u16,
    },
    /// A capability whose class is never required (component, raster, calibration) declared required.
    ComponentDeclaredRequired(CapabilityId),
    /// An optional extension declared required.
    ExtensionDeclaredRequired(CapabilityId),
    /// A block names a capability the manifest does not declare.
    UndeclaredBlockCapability {
        /// The block.
        block: BlockId,
        /// The capability it names.
        capability: u16,
    },
    /// A block of a kind a capability owns names another capability, or none.
    OwnedKindMismatch {
        /// The block.
        block: BlockId,
        /// Its kind id.
        kind: u16,
        /// The capability it names (0 for none).
        capability: u16,
    },
}

impl Refusal {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            Refusal::MissingRequired { .. } => "NEGOTIATE-01",
            Refusal::IncompatibleMajor { .. } => "NEGOTIATE-02",
            Refusal::ReaderTooOld { .. } => "NEGOTIATE-03",
            Refusal::ComponentDeclaredRequired(_) | Refusal::ExtensionDeclaredRequired(_) => "NEGOTIATE-04",
            Refusal::UndeclaredBlockCapability { .. } => "NEGOTIATE-05",
            Refusal::OwnedKindMismatch { .. } => "NEGOTIATE-06",
        }
    }
}

/// The refusals, rule by rule.
pub const NEGOTIATE_RULES: &[(&str, &str)] = &[
    (
        "NEGOTIATE-01",
        "a required capability is not supported by the reader, or not known to it at all; the refusal names its id and the version the file declares",
    ),
    (
        "NEGOTIATE-02",
        "a required capability is declared at a major version the reader does not support (a newer minor of a supported major opens)",
    ),
    (
        "NEGOTIATE-03",
        "the file needs a newer reader than this one (its minimum reader, or a required capability's)",
    ),
    (
        "NEGOTIATE-04",
        "a capability whose class is never required (a component, the raster or the calibration capability) or an optional extension is declared required, which the registry does not allow",
    ),
    ("NEGOTIATE-05", "a block names a capability id the manifest does not declare"),
    (
        "NEGOTIATE-06",
        "a block of a kind a capability owns (kinds 9 and 10 belong to capability 21, kind 11 belongs to capability 22) names another capability, or none",
    ),
];

/// The block kinds a capability owns, outside `BlockKind::ALL`: the kind id, its name and the owner
/// (ADR 0018, ADR 0019). A block of one of these kinds must name its owner in its directory entry (`NEGOTIATE-06`);
/// a reader that implements the owner is handed the block, and one that does not skips it as a kind it does
/// not know. `DIR-02` is untouched: a kind the format defines still names no capability.
pub const CAPABILITY_OWNED_KINDS: [(u16, &str, CapabilityId); 3] = [
    (9, "raster_index", CapabilityId::new(21)),
    (10, "raster_tiles", CapabilityId::new(21)),
    (11, "calibration", CapabilityId::new(22)),
];

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            Refusal::ReaderTooOld { file_requires, reader } => {
                write!(
                    f,
                    "{rule}: the file needs reader version {file_requires} or newer and this reader is {reader}"
                )
            }
            Refusal::MissingRequired { id, version, name: Some(n) } => {
                write!(
                    f,
                    "{rule}: required capability {n} (id {id}, version {version}) is not supported by this reader"
                )
            }
            Refusal::MissingRequired { id, version, name: None } => {
                write!(
                    f,
                    "{rule}: required capability with unknown id {id} (version {version}) is not supported by this reader"
                )
            }
            Refusal::IncompatibleMajor { id, file, supported_major } => write!(
                f,
                "{rule}: required capability id {id} is declared at version {file}, and this reader supports major version {supported_major}"
            ),
            Refusal::ComponentDeclaredRequired(id) => {
                write!(
                    f,
                    "{rule}: capability id {id} is of a class that is never required and cannot be declared required"
                )
            }
            Refusal::ExtensionDeclaredRequired(id) => {
                write!(
                    f,
                    "{rule}: capability id {id} is an optional extension and cannot be declared required"
                )
            }
            Refusal::UndeclaredBlockCapability { block, capability } => {
                write!(
                    f,
                    "{rule}: block {} names capability {capability}, which the manifest does not declare",
                    block.0
                )
            }
            Refusal::OwnedKindMismatch { block, kind, capability } => {
                write!(
                    f,
                    "{rule}: block {} is of kind {kind}, which belongs to another capability than the {capability} it names",
                    block.0
                )
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// Why something was skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The reader does not implement the capability.
    NotImplemented,
    /// The capability is declared at a newer major version than the reader supports.
    NewerMajor,
    /// A block of a kind this reader does not know.
    UnknownBlockKind,
    /// A block with a codec this reader does not know.
    UnknownCodec,
}

/// What a skipped capability leaves undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Entities are unstyled: layers and line styles.
    Unstyled,
    /// Entities lack a component: text, hatch, provenance, semantics.
    Uncomponented,
    /// The drawing has raster images this reader cannot show.
    Unrastered,
    /// The file carries a scale or a georeference this reader cannot show (ADR 0019).
    Uncalibrated,
    /// The reader cannot say.
    Unknown,
}

/// A capability that was skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedCapability {
    /// The id.
    pub id: CapabilityId,
    /// The version the file declares, when the manifest declares it.
    pub version: Option<Version>,
    /// The registry name, when known.
    pub name: Option<&'static str>,
    /// Why.
    pub reason: SkipReason,
    /// What it leaves undone.
    pub effect: Effect,
}

/// A block that was skipped, from its descriptor alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippedBlock {
    /// Where it is in the directory.
    pub block: BlockId,
    /// Its kind id.
    pub kind: u16,
    /// The capability it belongs to, when the descriptor names one.
    pub capability: Option<CapabilityId>,
    /// Its codec id.
    pub codec: u8,
    /// Its stored length.
    pub stored_length: u32,
    /// Why.
    pub reason: SkipReason,
}

/// What a degraded open left out, so a consumer can say so and not silently drop content.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PartialOpen {
    /// The capabilities skipped, ascending by id.
    pub capabilities: Vec<SkippedCapability>,
    /// The blocks skipped, in directory order.
    pub blocks: Vec<SkippedBlock>,
    /// The entities in the file, all of which a skipped capability may apply to. Which of them a skipped
    /// stream names is in its payload, which is not read.
    pub entities: u64,
}

/// Why a rewrite was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteRefused {
    /// The capability ids the rewrite would have dropped.
    pub skipped: Vec<u32>,
}

impl fmt::Display for RewriteRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a rewrite from the decoded model would drop capabilities {:?} that this reader did not understand",
            self.skipped
        )
    }
}

impl std::error::Error for RewriteRefused {}

impl PartialOpen {
    /// The ids of every skipped capability, ascending, each once: the manifest's and the blocks'.
    pub fn skipped_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.capabilities.iter().map(|c| c.id.get()).collect();
        ids.extend(self.blocks.iter().filter_map(|b| b.capability.map(CapabilityId::get)));
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Whether nothing was skipped.
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty() && self.blocks.is_empty()
    }

    /// The decision on re-writing: a rewrite that goes through the decoded model cannot carry what the
    /// reader did not decode, so it refuses when anything was skipped. A tool that must keep the blocks
    /// byte for byte copies them raw (`ContainerReader::read_stored` and `writer::assemble`), which
    /// preserves them exactly.
    pub fn ensure_lossless_rewrite(&self) -> Result<(), RewriteRefused> {
        if self.is_empty() { Ok(()) } else { Err(RewriteRefused { skipped: self.skipped_ids() }) }
    }

    /// A sentence a consumer can show.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "nothing was skipped".to_string();
        }
        let name = |c: &SkippedCapability| match c.name {
            Some(n) => format!("{} ({n})", c.id),
            None => c.id.to_string(),
        };
        let list = |effect: Effect| {
            self.capabilities.iter().filter(|c| c.effect == effect).map(name).collect::<Vec<_>>().join(", ")
        };
        let mut parts = Vec::new();
        let (styles, components, rasters, calibrations, unknown) = (
            list(Effect::Unstyled),
            list(Effect::Uncomponented),
            list(Effect::Unrastered),
            list(Effect::Uncalibrated),
            list(Effect::Unknown),
        );
        if !styles.is_empty() {
            parts.push(format!(
                "the {} entities may be unstyled, because capability {styles} was not read",
                self.entities
            ));
        }
        if !components.is_empty() {
            parts.push(format!(
                "the {} entities may lack components, because capability {components} was not read",
                self.entities
            ));
        }
        if !rasters.is_empty() {
            parts.push(format!(
                "this file has an image this reader cannot show, because capability {rasters} was not read"
            ));
        }
        if !calibrations.is_empty() {
            parts.push(format!(
                "this file carries a scale or georeference this reader cannot show, because capability {calibrations} was not read"
            ));
        }
        if !unknown.is_empty() {
            parts.push(format!("capability {unknown} is unknown to this reader and was not read"));
        }
        if !self.blocks.is_empty() {
            let ids: Vec<String> = self.blocks.iter().map(|b| b.block.0.to_string()).collect();
            parts.push(format!(
                "{} block(s) were skipped without being read (block {})",
                self.blocks.len(),
                ids.join(", ")
            ));
        }
        let mut s = parts.join("; ");
        s.push('.');
        s
    }
}

/// The outcome of negotiating a file against a reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every capability is supported.
    Open,
    /// Optional capabilities are not supported and their blocks are skipped.
    OpenDegraded(PartialOpen),
    /// A required capability is not supported.
    Refuse(Refusal),
}

fn effect_of(id: CapabilityId) -> Effect {
    match registry().get(id) {
        Some(c) if c.class == Class::Component => {
            if [
                pvec_capabilities::ids::LAYER,
                pvec_capabilities::ids::LINE_STYLE,
                pvec_capabilities::ids::STROKE_PAINT,
            ]
            .contains(&id)
            {
                Effect::Unstyled
            } else {
                Effect::Uncomponented
            }
        }
        Some(c) if c.class == Class::Raster => Effect::Unrastered,
        Some(c) if c.class == Class::Calibration => Effect::Uncalibrated,
        _ => Effect::Unknown,
    }
}

/// Negotiates `manifest` and the block directory against `reader`.
///
/// Reads only what the manifest and the descriptors say: no payload is looked at, so what is skipped is
/// skipped from its descriptor alone.
pub fn negotiate(manifest: &Manifest, entries: &[BlockEntry], reader: &ReaderCapabilities) -> Outcome {
    if manifest.min_reader > reader.version {
        return Outcome::Refuse(Refusal::ReaderTooOld {
            file_requires: manifest.min_reader,
            reader: reader.version,
        });
    }
    let reg = registry();
    let mut skipped: Vec<SkippedCapability> = Vec::new();
    for d in &manifest.capabilities {
        let known = reg.get(d.id);
        if let Some(c) = known {
            // only a required capability can make the file need a newer reader: an optional one is skippable by
            // definition, which is the same reason the writer's hoist leaves it out of `min_reader`
            if c.min_reader > reader.version
                && (d.requirement == Requirement::Required || c.requirement == Requirement::Required)
            {
                return Outcome::Refuse(Refusal::ReaderTooOld {
                    file_requires: c.min_reader,
                    reader: reader.version,
                });
            }
            if matches!(c.class, Class::Component | Class::Raster | Class::Calibration)
                && d.requirement == Requirement::Required
            {
                return Outcome::Refuse(Refusal::ComponentDeclaredRequired(d.id));
            }
            if c.class == Class::Extension
                && c.requirement == Requirement::Optional
                && d.requirement == Requirement::Required
            {
                return Outcome::Refuse(Refusal::ExtensionDeclaredRequired(d.id));
            }
        }
        let required = d.requirement == Requirement::Required
            || known.is_some_and(|c| c.requirement == Requirement::Required);
        let skip = |reason| SkippedCapability {
            id: d.id,
            version: Some(d.version),
            name: known.map(|c| c.name),
            reason,
            effect: effect_of(d.id),
        };
        match reader.supports(d.id) {
            None if required => {
                return Outcome::Refuse(Refusal::MissingRequired {
                    id: d.id,
                    version: d.version,
                    name: known.map(|c| c.name),
                });
            }
            None => skipped.push(skip(SkipReason::NotImplemented)),
            Some((major, _)) if d.version.major != major => {
                if required {
                    return Outcome::Refuse(Refusal::IncompatibleMajor {
                        id: d.id,
                        file: d.version,
                        supported_major: major,
                    });
                }
                skipped.push(skip(if d.version.major > major {
                    SkipReason::NewerMajor
                } else {
                    SkipReason::NotImplemented
                }));
            }
            // a newer minor of a supported major only adds, so the file opens
            Some(_) => {}
        }
    }

    // looked up by id for every block, so it is a map: a manifest and a directory can both be large
    let by_id: std::collections::HashMap<u32, &Declared> =
        manifest.capabilities.iter().map(|d| (d.id.get(), d)).collect();
    let declared = |id: u32| by_id.get(&id).copied();
    let mut skipped_reason: std::collections::HashMap<u32, SkipReason> =
        skipped.iter().map(|s| (s.id.get(), s.reason)).collect();
    let mut blocks: Vec<SkippedBlock> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let block = BlockId(i as u32);
        let capability = (e.capability != 0).then(|| CapabilityId::new(u32::from(e.capability)));
        if let Some(c) = capability {
            if declared(c.get()).is_none() {
                return Outcome::Refuse(Refusal::UndeclaredBlockCapability {
                    block,
                    capability: e.capability,
                });
            }
        }
        let by_capability = capability.and_then(|c| skipped_reason.get(&c.get()).copied());
        // a kind a capability owns (CONTAINER.md 5.4 item 2): handed to the owner when this reader implements
        // it, skipped as a kind it does not know otherwise
        let owner = CAPABILITY_OWNED_KINDS.iter().find(|(k, _, _)| *k == e.kind).map(|(_, _, o)| *o);
        if let Some(owner) = owner {
            if capability != Some(owner) {
                return Outcome::Refuse(Refusal::OwnedKindMismatch {
                    block,
                    kind: e.kind,
                    capability: e.capability,
                });
            }
        }
        let reason = if e.block_kind().is_none() && !owner.is_some_and(|o| reader.supports(o).is_some()) {
            Some(SkipReason::UnknownBlockKind)
        } else if crate::compress::Codec::from_id(e.codec).is_none() {
            Some(SkipReason::UnknownCodec)
        } else {
            by_capability
        };
        if let Some(reason) = reason {
            blocks.push(SkippedBlock {
                block,
                kind: e.kind,
                capability,
                codec: e.codec,
                stored_length: e.stored_length,
                reason,
            });
            // a skipped block's capability is in the report even when the reader implements it
            if let Some(c) = capability {
                if skipped_reason.insert(c.get(), reason).is_none() {
                    let d = declared(c.get());
                    skipped.push(SkippedCapability {
                        id: c,
                        version: d.map(|d| d.version),
                        name: reg.get(c).map(|r| r.name),
                        reason,
                        effect: effect_of(c),
                    });
                }
            }
        }
    }
    if skipped.is_empty() && blocks.is_empty() {
        return Outcome::Open;
    }
    skipped.sort_by_key(|s| s.id);
    let entities = entries
        .iter()
        .filter(|e| matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records)))
        .map(|e| u64::from(e.count))
        .sum();
    Outcome::OpenDegraded(PartialOpen { capabilities: skipped, blocks, entities })
}

/// A capability the file declares at a newer minor version than the reader implements. The reader reads
/// the subset it understands, since a minor only adds, and reports the gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionGap {
    /// The capability.
    pub id: CapabilityId,
    /// The version the file declares.
    pub file: Version,
    /// The `(major, minor)` the reader implements.
    pub supported: (u16, u16),
}

/// The capabilities `manifest` declares at a newer minor version of a major the reader implements,
/// ascending by id.
pub fn version_gaps(manifest: &Manifest, reader: &ReaderCapabilities) -> Vec<VersionGap> {
    let mut gaps: Vec<VersionGap> = manifest
        .capabilities
        .iter()
        .filter_map(|d| {
            let (major, minor) = reader.supports(d.id)?;
            (d.version.major == major && d.version.minor > minor).then_some(VersionGap {
                id: d.id,
                file: d.version,
                supported: (major, minor),
            })
        })
        .collect();
    gaps.sort_by_key(|g| g.id);
    gaps
}

/// Chapter 5 of `docs/CONTAINER.md`, rendered from the constants.
pub(crate) fn render_chapter() -> String {
    let mut s = String::new();
    s.push_str(
        "## 5. Negotiation and skipping blocks\n\n\
A reader that meets a block it has never heard of has to skip it correctly, without parsing its payload, or \
every future capability is a breaking change. This chapter is the contract that makes optional capabilities \
additive. Negotiation is capability-based and not a single version number (ADR 0008), so it is about sets: \
what the manifest declares, and what a reader supports.\n\n\
### 5.1 What a reader supports\n\n\
`ReaderCapabilities` is the set and the versions a build supports: for each capability id, the major version it \
implements and the newest minor of it. A file declares each capability it uses in the manifest (chapter 1) with \
the version it was written at and a requirement, required or optional. Matching them gives one of three outcomes.\n\n\
### 5.2 The three outcomes\n\n\
| outcome | when | what the reader does |\n|---|---|---|\n\
| `Open` | every declared capability is supported, at a major version the reader implements | reads the file |\n\
| `OpenDegraded` | an optional capability is not supported, or is declared at a major version the reader does not implement, or a block is of a kind or a codec the reader does not know | reads the file, skips what it cannot read, and returns a `PartialOpen` that lists the skipped capability ids and blocks |\n\
| `Refuse` | a required capability is not supported, or is declared at another major version, or the file needs a newer reader | opens nothing, and the refusal names the capability id and the version the file declares |\n\n\
A newer minor version of a capability the reader supports opens: a minor only adds, and what it adds is ignorable \
by the growth rules of section 1.7. `version_gaps` lists those capabilities with the version the file declares and \
the one the reader implements, so a consumer can report the gap. A newer major of an optional capability is skipped, and of a required one is \
refused. The registry's own rules apply first: a capability whose class is never required (component, raster, calibration) cannot be declared required, and an extension the registry lists as \
optional cannot be declared required by a file.\n\n\
Refusals are `Refusal` values and carry a rule:\n\n\
| rule | meaning |\n|---|---|\n",
    );
    for (rule, meaning) in NEGOTIATE_RULES {
        s.push_str(&format!("| {rule} | {meaning} |\n"));
    }
    s.push_str(
        "\n### 5.3 The skip contract\n\n\
A block carries everything needed to skip it in its directory entry: its kind id, the capability id it belongs to \
(0 for the kinds this format defines), its offset and stored length, its codec id and its decoded length. A reader \
skips a block from a descriptor alone, and never requests a byte of its payload:\n\n\
* a block of a kind the reader does not know is skipped when its must-understand bit is clear, and refuses the file \
when it is set (`DIR-04`);\n\
* a block with a codec the reader does not know is skipped on the same terms (`DIR-05`);\n\
* a block whose capability the reader skipped is skipped with it;\n\
* a block of a kind a capability owns (kinds 9 and 10, which belong to capability 21, and kind 11, which belongs to \
capability 22; none is in `BlockKind::ALL`) is handed to that capability when the reader implements it, and is skipped as a kind the reader does not \
know, naming the capability from its entry, when it does not. It must name its owner and no other capability \
(`NEGOTIATE-06`).\n\n\
A skipped block is not fetched, not checksummed, not decompressed, not parsed and not counted in a read plan, and \
a coalesced request never reads across it. The one exception is the explicit whole-file digest verification, which \
reads every byte by definition. The \
reader reports it, and the entity counts and the geometry it reads are bit-identical to the same file without the \
block. A block cannot name a capability the manifest does not declare (`NEGOTIATE-05`), and a block of a kind the \
format defines names none (`DIR-02`), because its kind fixes it.\n\n\
### 5.4 What a new capability must satisfy to be optional\n\n\
A capability is optional, and so skippable by an old reader, only if it satisfies every one of these. One it cannot \
satisfy is required, and an old reader refuses the file, which is the right outcome.\n\n\
1. It adds information about the drawing and never changes what a geometry block says. Style, text, hatch and \
provenance are the examples; a new geometry kind is not, it needs an ADR and a section of `docs/ISO_PROFILE.md` \
(ADR 0008) and is always required.\n\
2. Every block it writes has the must-understand bit clear, and the descriptor of each names the capability id. This path is first used by capability 21 (`raster`, kinds 9 and 10), which is why DIR-02 reads \"a kind the format defines\".\n\
3. The manifest declares it with the optional requirement and the version it was written at.\n\
4. Its blocks are delimited by their descriptors alone: a reader never needs to read a payload to know where it ends.\n\
5. Nothing a reader that does skip it must read points into it: no geometry block, record or arena refers to a block of \
the capability.\n\
6. Its id comes from the registry (`docs/CAPABILITIES.md`, `docs/EXTENSIONS.md`) and is never reused.\n\n\
### 5.5 The report and re-writing\n\n\
`PartialOpen` lists the skipped capability ids with their versions, names and what each leaves undone (entities \
unstyled, entities without a component, an image or a calibration the reader cannot show, or unknown), the skipped blocks, and the number of entities a skipped \
capability may apply to, so a consumer can say so and not silently drop content. Which entities a skipped stream \
names is in its payload, which is not read.\n\n\
Re-writing a file whose optional blocks the reader did not understand has one answer per path. A rewrite that goes \
through the decoded model refuses (`PartialOpen::ensure_lossless_rewrite`), because the model cannot carry what \
was not decoded. A tool that must keep the blocks copies them as they are, through `read_stored` and `assemble`, and \
preserves them byte for byte, the descriptor and the checksum included. Both are tested.\n\n",
    );
    s
}
