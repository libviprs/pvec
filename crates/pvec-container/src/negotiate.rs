//! Capability negotiation and unknown-block skipping (issue P4.7). STUB for the red commit.
#![allow(dead_code)]

use crate::directory::BlockEntry;
use crate::manifest::Manifest;
use crate::reader::BlockId;
use pvec_capabilities::{CapabilityId, Registry, Version};
use std::fmt;

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
        let registry = Registry::v1();
        let supported =
            registry.entries().iter().map(|c| (c.id, c.version.major, c.version.minor)).collect::<Vec<_>>();
        ReaderCapabilities { version: Version::new(1, 0, 0), supported }
    }

    /// A reader of registry version `version` that supports nothing yet.
    pub fn none(version: Version) -> ReaderCapabilities {
        ReaderCapabilities { version, supported: Vec::new() }
    }

    /// Supports capability `id` up to `major.minor`.
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
    /// A component declared required, which components never are.
    ComponentDeclaredRequired(CapabilityId),
    /// An optional extension declared required.
    ExtensionDeclaredRequired(CapabilityId),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
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
    /// The capabilities skipped, by id.
    pub capabilities: Vec<SkippedCapability>,
    /// The blocks skipped.
    pub blocks: Vec<SkippedBlock>,
    /// The entities in the file, all of which a skipped capability may apply to.
    pub entities: u64,
}

/// Why a rewrite was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteRefused {
    /// The capability ids the rewrite would have dropped.
    pub skipped: Vec<u32>,
}

impl PartialOpen {
    /// The ids of every skipped capability, ascending, each once.
    pub fn skipped_ids(&self) -> Vec<u32> {
        Vec::new()
    }

    /// Whether nothing was skipped.
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty() && self.blocks.is_empty()
    }

    /// Succeeds only when nothing was skipped.
    pub fn ensure_lossless_rewrite(&self) -> Result<(), RewriteRefused> {
        Ok(())
    }

    /// A sentence a consumer can show.
    pub fn describe(&self) -> String {
        String::new()
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

/// Negotiates `manifest` and the block directory against `reader`.
pub fn negotiate(_manifest: &Manifest, _entries: &[BlockEntry], _reader: &ReaderCapabilities) -> Outcome {
    Outcome::Open
}
