//! The registry snapshot and the compatibility lint (issue P7.6).
//!
//! `docs/capabilities.toml` is the machine-readable snapshot of the registry. [`lint_transition`] compares
//! two snapshots, the one on the base branch and the one in a change, and reports every transition the
//! compatibility rules of `docs/COMPATIBILITY.md` forbid.

use crate::{Registry, Version};

/// The version of the format (container and registry together) this build writes.
pub const FORMAT_VERSION: Version = Version::new(1, 0, 0);

/// Minor releases a capability stays `deprecated` before it may become `write-disabled`.
pub const DEPRECATED_MIN_MINORS: u16 = 2;

/// Minor releases a capability stays `write-disabled` before it may be removed.
pub const WRITE_DISABLED_MIN_MINORS: u16 = 2;

/// Where a capability is in the deprecation lifecycle. `removed` is not a status: a removed capability
/// leaves the entry list and joins the retired list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Written and read.
    Active,
    /// Still written and read, and announced for removal.
    Deprecated,
    /// Read only: a writer must not produce it.
    WriteDisabled,
}

/// One capability of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// The id.
    pub id: u32,
    /// The name.
    pub name: String,
    /// `geometry`, `component` or `extension`.
    pub class: String,
    /// The capability's own version.
    pub version: Version,
    /// `required` or `optional`.
    pub requirement: String,
    /// The lowest reader version that understands it.
    pub min_reader: Version,
    /// Where it is in the lifecycle.
    pub status: Status,
    /// The format version at which it entered that status.
    pub status_since: Version,
}

/// A parsed `capabilities.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The format version the snapshot describes.
    pub format_version: Version,
    /// The live capabilities, ascending by id.
    pub entries: Vec<SnapshotEntry>,
    /// The retired ids and names.
    pub retired: Vec<(u32, String)>,
}

impl Snapshot {
    /// The snapshot of a registry: every entry active since 1.0.0.
    pub fn from_registry(_registry: &Registry) -> Snapshot {
        todo!("P7.6")
    }

    /// Parses the TOML subset the snapshot is written in.
    pub fn parse(_text: &str) -> Result<Snapshot, String> {
        todo!("P7.6")
    }

    /// The TOML text. Deterministic; ends in one newline.
    pub fn render(&self) -> String {
        todo!("P7.6")
    }
}

/// The kind of breaking change a minor format version may not carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breaking {
    /// A capability left the registry.
    Removal,
    /// A capability's major version went up.
    MajorBump,
    /// An optional capability became required.
    RequirementTightened,
    /// The lowest reader that understands a capability went up.
    MinReaderRaised,
    /// A capability changed class.
    ClassChanged,
}

/// One transition the compatibility rules forbid.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LintFinding {
    /// The format version of the change is below the base's.
    FormatVersionWentDown {
        /// On the base branch.
        old: Version,
        /// In the change.
        new: Version,
    },
    /// A capability was removed without being `write-disabled` first.
    RemovedWithoutLifecycle {
        /// The id.
        id: u32,
        /// Its status on the base branch.
        last_status: Status,
    },
    /// A removed capability's id is not on the retired list, so it could be handed out again.
    RemovedNotRetired {
        /// The id.
        id: u32,
    },
    /// A status moved forward by more than one step.
    StatusSkipped {
        /// The id.
        id: u32,
        /// Before.
        from: Status,
        /// After.
        to: Status,
    },
    /// A status moved backwards.
    StatusRegressed {
        /// The id.
        id: u32,
        /// Before.
        from: Status,
        /// After.
        to: Status,
    },
    /// A capability left a status before it had been in it for the minimum number of minor releases.
    LeftStatusTooEarly {
        /// The id.
        id: u32,
        /// The status it left.
        status: Status,
        /// The format version it entered it at.
        since: Version,
        /// The minor releases required.
        needed_minors: u16,
    },
    /// A breaking change on a format version whose major did not go up.
    BreakingOnMinor {
        /// The id.
        id: u32,
        /// What changed.
        change: Breaking,
    },
    /// A capability's version went down.
    VersionWentDown {
        /// The id.
        id: u32,
        /// Before.
        old: Version,
        /// After.
        new: Version,
    },
    /// A live capability uses an id on the retired list.
    RetiredIdReused {
        /// The id.
        id: u32,
    },
}

/// Compares the snapshot on the base branch with the one in a change.
pub fn lint_transition(_old: &Snapshot, _new: &Snapshot) -> Vec<LintFinding> {
    todo!("P7.6")
}
