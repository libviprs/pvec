//! The registry snapshot and the compatibility lint (issue P7.6).
//!
//! `docs/capabilities.toml` is the machine-readable snapshot of the registry. [`lint_transition`] compares
//! two snapshots, the one on the base branch and the one in a change, and reports every transition the
//! compatibility rules of `docs/COMPATIBILITY.md` forbid.

use crate::{Registry, Version};

/// The version of the format (container and registry together) this build writes.
pub const FORMAT_VERSION: Version = Version::new(1, 3, 0);

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

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Deprecated => "deprecated",
            Status::WriteDisabled => "write-disabled",
        }
    }

    /// The minor releases a capability must spend in this status before it moves on.
    fn min_minors(self) -> u16 {
        match self {
            Status::Active => 0,
            Status::Deprecated => DEPRECATED_MIN_MINORS,
            Status::WriteDisabled => WRITE_DISABLED_MIN_MINORS,
        }
    }
}

impl Snapshot {
    /// The snapshot of a registry: every entry active, since the format version whose readers first understand it.
    pub fn from_registry(registry: &Registry) -> Snapshot {
        let mut entries: Vec<SnapshotEntry> = registry
            .entries()
            .iter()
            .map(|c| SnapshotEntry {
                id: c.id.get(),
                name: c.name.to_string(),
                class: c.class.label().to_string(),
                version: c.version,
                requirement: c.requirement.label().to_string(),
                min_reader: c.min_reader,
                status: Status::Active,
                status_since: c.min_reader,
            })
            .collect();
        entries.sort_by_key(|e| e.id);
        let retired = registry.retired().iter().map(|r| (r.id.get(), r.name.to_string())).collect();
        Snapshot { format_version: FORMAT_VERSION, entries, retired }
    }

    /// Parses the TOML subset the snapshot is written in: `key = "text"` and `key = number` lines under
    /// `[[capability]]` and `[[retired]]` headers, and `#` comments.
    pub fn parse(text: &str) -> Result<Snapshot, String> {
        enum In {
            Top,
            Capability,
            Retired,
        }
        let version = |v: &str| -> Result<Version, String> {
            let parts: Vec<&str> = v.split('.').collect();
            let [a, b, c] = parts[..] else { return Err(format!("not a major.minor.patch version: {v:?}")) };
            let n = |s: &str| s.parse::<u16>().map_err(|_| format!("not a version number: {v:?}"));
            Ok(Version::new(n(a)?, n(b)?, n(c)?))
        };
        let mut format_version = None;
        let mut entries: Vec<SnapshotEntry> = Vec::new();
        let mut retired: Vec<(u32, String)> = Vec::new();
        let mut section = In::Top;
        let mut open: Vec<(String, String)> = Vec::new();

        // a section is complete when the next header or the end of the text arrives
        let mut flush = |section: &In, open: &mut Vec<(String, String)>| -> Result<(), String> {
            let get = |k: &str| -> Result<String, String> {
                open.iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.clone())
                    .ok_or(format!("missing key {k}"))
            };
            let id = |open: &Vec<(String, String)>| -> Result<u32, String> {
                let v =
                    open.iter().find(|(k, _)| k == "id").map(|(_, v)| v.as_str()).ok_or("missing key id")?;
                v.parse().map_err(|_| format!("id is not a number: {v:?}"))
            };
            match section {
                In::Top => {}
                In::Capability => {
                    let status = match get("status")?.as_str() {
                        "active" => Status::Active,
                        "deprecated" => Status::Deprecated,
                        "write-disabled" => Status::WriteDisabled,
                        other => return Err(format!("unknown status {other:?}")),
                    };
                    entries.push(SnapshotEntry {
                        id: id(open)?,
                        name: get("name")?,
                        class: get("class")?,
                        version: version(&get("version")?)?,
                        requirement: get("requirement")?,
                        min_reader: version(&get("min_reader")?)?,
                        status,
                        status_since: version(&get("status_since")?)?,
                    });
                }
                In::Retired => retired.push((id(open)?, get("name")?)),
            }
            open.clear();
            Ok(())
        };

        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: String| format!("line {}: {e}", n + 1);
            match line {
                "[[capability]]" | "[[retired]]" => {
                    flush(&section, &mut open).map_err(at)?;
                    section = if line == "[[capability]]" { In::Capability } else { In::Retired };
                }
                _ if line.starts_with('[') => return Err(at(format!("unknown table {line}"))),
                _ => {
                    let (k, v) =
                        line.split_once('=').ok_or_else(|| at(format!("not a key = value line: {line}")))?;
                    let v = v.trim();
                    let value = match v.strip_prefix('"') {
                        Some(rest) => rest
                            .strip_suffix('"')
                            .ok_or_else(|| at("unterminated string".into()))?
                            .to_string(),
                        None if v.parse::<u32>().is_ok() => v.to_string(),
                        None => return Err(at(format!("a string needs quotes: {v}"))),
                    };
                    if matches!(section, In::Top) {
                        if k.trim() != "format_version" {
                            return Err(at(format!("unknown top-level key {}", k.trim())));
                        }
                        format_version = Some(version(&value).map_err(at)?);
                    } else {
                        open.push((k.trim().to_string(), value));
                    }
                }
            }
        }
        flush(&section, &mut open).map_err(|e| format!("end of file: {e}"))?;
        let format_version = format_version.ok_or("missing format_version")?;
        Ok(Snapshot { format_version, entries, retired })
    }

    /// The TOML text. Deterministic; ends in one newline.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# The capability registry snapshot. Generated from crates/pvec-capabilities; I do not edit it by hand.\n\
             # Regenerate with: PVEC_BLESS_CAPABILITIES=1 cargo test -p pvec-checks --test compatibility_doc\n\
             # The rules it is linted against are in docs/COMPATIBILITY.md.\n",
        );
        out.push_str(&format!("format_version = \"{}\"\n", self.format_version));
        for e in &self.entries {
            out.push_str(&format!(
                "\n[[capability]]\nid = {}\nname = \"{}\"\nclass = \"{}\"\nversion = \"{}\"\n\
                 requirement = \"{}\"\nmin_reader = \"{}\"\nstatus = \"{}\"\nstatus_since = \"{}\"\n",
                e.id,
                e.name,
                e.class,
                e.version,
                e.requirement,
                e.min_reader,
                e.status.name(),
                e.status_since
            ));
        }
        for (id, name) in &self.retired {
            out.push_str(&format!("\n[[retired]]\nid = {id}\nname = \"{name}\"\n"));
        }
        out
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

/// Minor releases between the format version a status was entered at and `now`. A new major version
/// counts as long enough, since it is a release of its own.
fn minors_between(since: Version, now: Version) -> u16 {
    if now.major > since.major { u16::MAX } else { now.minor.saturating_sub(since.minor) }
}

/// Compares the snapshot on the base branch with the one in a change.
pub fn lint_transition(old: &Snapshot, new: &Snapshot) -> Vec<LintFinding> {
    let mut found = Vec::new();
    if new.format_version < old.format_version {
        found.push(LintFinding::FormatVersionWentDown { old: old.format_version, new: new.format_version });
    }
    let major_bump = new.format_version.major > old.format_version.major;
    let breaking = |id: u32, change: Breaking, found: &mut Vec<LintFinding>| {
        if !major_bump {
            found.push(LintFinding::BreakingOnMinor { id, change });
        }
    };
    for o in &old.entries {
        let Some(n) = new.entries.iter().find(|n| n.id == o.id) else {
            if o.status != Status::WriteDisabled {
                found.push(LintFinding::RemovedWithoutLifecycle { id: o.id, last_status: o.status });
            } else if minors_between(o.status_since, new.format_version) < o.status.min_minors() {
                found.push(LintFinding::LeftStatusTooEarly {
                    id: o.id,
                    status: o.status,
                    since: o.status_since,
                    needed_minors: o.status.min_minors(),
                });
            }
            if !new.retired.iter().any(|(id, _)| *id == o.id) {
                found.push(LintFinding::RemovedNotRetired { id: o.id });
            }
            breaking(o.id, Breaking::Removal, &mut found);
            continue;
        };
        if n.status < o.status {
            found.push(LintFinding::StatusRegressed { id: o.id, from: o.status, to: n.status });
        } else if n.status > o.status {
            if n.status as u8 - o.status as u8 > 1 {
                found.push(LintFinding::StatusSkipped { id: o.id, from: o.status, to: n.status });
            }
            if minors_between(o.status_since, new.format_version) < o.status.min_minors() {
                found.push(LintFinding::LeftStatusTooEarly {
                    id: o.id,
                    status: o.status,
                    since: o.status_since,
                    needed_minors: o.status.min_minors(),
                });
            }
        }
        if n.version < o.version {
            found.push(LintFinding::VersionWentDown { id: o.id, old: o.version, new: n.version });
        }
        if n.version.major > o.version.major {
            breaking(o.id, Breaking::MajorBump, &mut found);
        }
        if o.requirement == "optional" && n.requirement == "required" {
            breaking(o.id, Breaking::RequirementTightened, &mut found);
        }
        if n.min_reader > o.min_reader {
            breaking(o.id, Breaking::MinReaderRaised, &mut found);
        }
        if n.class != o.class {
            breaking(o.id, Breaking::ClassChanged, &mut found);
        }
    }
    for n in &new.entries {
        if new.retired.iter().any(|(id, _)| *id == n.id) || old.retired.iter().any(|(id, _)| *id == n.id) {
            found.push(LintFinding::RetiredIdReused { id: n.id });
        }
    }
    found
}
