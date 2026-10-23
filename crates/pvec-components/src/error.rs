//! Typed refusals for the stream framing and for the payloads. Each carries a rule identifier:
//! `STREAM-nn` for the framing and `COMP-nn` for a payload, as listed in `docs/COMPONENT_STREAMS.md`.

use core::fmt;
use pvec_capabilities::CapabilityId;
use pvec_entity::EntityId;

/// Why a set of streams was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamError {
    /// The first four bytes are not `PVCS`.
    BadMagic,
    /// A set version this reader does not understand.
    UnsupportedVersion(u16),
    /// A reserved field is not zero.
    ReservedNotZero,
    /// The bytes end before the structure does.
    Truncated,
    /// More streams than the limit.
    TooMany,
    /// A padding byte is not zero.
    NonZeroPadding,
    /// Bytes remain after the last stream.
    TrailingBytes,
    /// A stream declares a geometry capability id. Component streams never do.
    GeometryCapability(CapabilityId),
    /// A stream declares the id of something that is not a component.
    NotAComponent(CapabilityId),
    /// A payload is too large for the 32-bit length of a frame.
    TooLarge,
}

impl StreamError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            StreamError::BadMagic => "STREAM-01",
            StreamError::UnsupportedVersion(_) => "STREAM-02",
            StreamError::ReservedNotZero => "STREAM-03",
            StreamError::Truncated => "STREAM-04",
            StreamError::TooMany => "STREAM-05",
            StreamError::NonZeroPadding => "STREAM-06",
            StreamError::TrailingBytes => "STREAM-07",
            StreamError::GeometryCapability(_) => "STREAM-08",
            StreamError::NotAComponent(_) => "STREAM-09",
            StreamError::TooLarge => "STREAM-10",
        }
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.rule();
        match self {
            StreamError::BadMagic => write!(f, "not a component stream set: bad magic ({r})"),
            StreamError::UnsupportedVersion(v) => write!(f, "unsupported stream set version {v} ({r})"),
            StreamError::ReservedNotZero => write!(f, "a reserved field is not zero ({r})"),
            StreamError::Truncated => {
                write!(f, "the stream set is shorter than its structure requires ({r})")
            }
            StreamError::TooMany => write!(f, "more streams than the limit ({r})"),
            StreamError::NonZeroPadding => write!(f, "padding is not zero ({r})"),
            StreamError::TrailingBytes => write!(f, "bytes remain after the last stream ({r})"),
            StreamError::GeometryCapability(id) => {
                write!(f, "a component stream declares the geometry capability id {id} ({r})")
            }
            StreamError::NotAComponent(id) => write!(f, "capability id {id} is not a component ({r})"),
            StreamError::TooLarge => write!(f, "a payload does not fit a 32-bit length ({r})"),
        }
    }
}

impl std::error::Error for StreamError {}

/// Why a payload was refused, on the way out or on the way in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ComponentError {
    /// The payload ends before its structure does.
    Truncated,
    /// Bytes remain after the structure.
    Trailing,
    /// A count above its limit.
    TooMany,
    /// A string or byte run above its limit.
    TooLong,
    /// A count that must not be zero is.
    BadCount,
    /// A number that is not finite, or not positive where it has to be.
    BadNumber,
    /// A string that is not valid UTF-8.
    BadUtf8,
    /// An entity id at or above the limit.
    BadEntityId(u32),
    /// A layer or style index with no entry in the table.
    IndexOutOfRange,
    /// A hatch that carries curve parameters instead of entity references.
    EmbeddedGeometry,
    /// A tag the format does not define.
    BadTag(u32),
    /// A reserved field is not zero.
    ReservedNotZero,
    /// The registry has no entry for the stream's capability.
    UnknownCapability(CapabilityId),
    /// An entity appears twice where it may appear once.
    Duplicate(EntityId),
}

impl ComponentError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            ComponentError::Truncated => "COMP-01",
            ComponentError::Trailing => "COMP-02",
            ComponentError::TooMany => "COMP-03",
            ComponentError::TooLong => "COMP-04",
            ComponentError::BadCount => "COMP-05",
            ComponentError::BadNumber => "COMP-06",
            ComponentError::BadUtf8 => "COMP-07",
            ComponentError::BadEntityId(_) => "COMP-08",
            ComponentError::IndexOutOfRange => "COMP-09",
            ComponentError::EmbeddedGeometry => "COMP-10",
            ComponentError::BadTag(_) => "COMP-11",
            ComponentError::ReservedNotZero => "COMP-12",
            ComponentError::UnknownCapability(_) => "COMP-13",
            ComponentError::Duplicate(_) => "COMP-14",
        }
    }
}

impl fmt::Display for ComponentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?} ({})", self.rule())
    }
}

impl std::error::Error for ComponentError {}
