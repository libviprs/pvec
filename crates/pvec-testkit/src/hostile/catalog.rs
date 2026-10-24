//! The cases. One named case per attack, each a mutation of a valid base.

use super::bases::Base;
use super::mutate::{Op, apply};

/// What a case needs from a reader beyond reading the parts of the format that already have parsers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Requires {
    /// Only what the header, manifest, directory and block parsers refuse.
    Parse,
    /// A resource budget (P4.8): an expansion ratio, a decoded-size ceiling.
    Limits,
    /// Per-block checksums and the artifact digest (P4.6).
    Integrity,
    /// Skipping an optional block the reader does not know (P4.7).
    Negotiation,
}

/// The case families of the issue, plus the ones it implies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
    /// Offset and length wrap, past the end of the file, overlapping blocks.
    Offsets,
    /// Entity, control-point, knot and record counts that lie.
    Counts,
    /// An expansion ratio beyond the budget.
    Decompression,
    /// A composite that names itself, and nesting that is too deep.
    Nesting,
    /// Knot vectors that are not monotonic, the wrong length, or repeat too often.
    Knots,
    /// Zero, negative, non-finite or missing weights.
    Weights,
    /// Inverted, zero-length or non-finite domains and coordinates.
    Domains,
    /// A wrong artifact digest or block checksum.
    Integrity,
    /// A file cut short, or with a byte removed.
    Truncation,
    /// Bytes after the end of the artifact.
    TrailingBytes,
    /// A kind, codec, version, flag or algorithm the reader does not know.
    UnknownTags,
    /// Order, padding, reserved fields and length fields that are wrong inside a block.
    Corruption,
}

/// What a reader has to do with the mutated file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Refuse, naming one of these rules.
    Refuse(Vec<&'static str>),
    /// Accept: the mutation is something a reader must tolerate.
    Accept,
}

/// One attack.
#[derive(Clone, Debug)]
pub struct Case {
    /// A unique name, `family/what`, or `family/what/base/position` for the generated sweeps.
    pub name: String,
    /// The family.
    pub family: Family,
    /// What a reader needs to provide to be held to it.
    pub requires: Requires,
    /// Index into the bases.
    pub base: usize,
    /// The mutation, as byte-level edits of the base.
    pub ops: Vec<Op>,
    /// What the reader has to do.
    pub expect: Expect,
}

impl Case {
    /// The mutated file.
    pub fn bytes(&self, base: &Base) -> Vec<u8> {
        apply(&base.bytes, &self.ops)
    }
}

/// Every case over these bases.
pub fn catalog(_bases: &[Base]) -> Vec<Case> {
    Vec::new()
}
