//! The framing of a stream set: a header and one frame per stream. A frame says which capability it
//! belongs to, at which version, and how long its payload is, which is all a reader needs to step over a
//! stream it does not understand without looking inside.

use crate::error::StreamError;
use pvec_capabilities::{CapabilityId, Class, Reader, Registry, Version};
use std::sync::OnceLock;

const MAGIC: [u8; 4] = *b"PVCS";
const SET_VERSION: u16 = 1;
const SET_HEADER: usize = 16;
const FRAME_HEADER: usize = 16;
/// The most streams one set may hold. The registry has six; the rest is room for unknown ones.
pub const MAX_STREAMS: usize = 4096;

/// One component stream: its capability, the version it was written at, and its raw payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    /// The registry capability.
    pub capability: CapabilityId,
    /// The semantic version of the stream's payload format.
    pub version: Version,
    /// The payload, which the typed streams of this crate encode and decode.
    pub payload: Vec<u8>,
}

/// Why a stream was set aside and not returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The registry has no such capability, so this reader cannot know what the payload is.
    UnknownCapability,
    /// The capability is known but the reader does not implement it.
    NotImplemented,
    /// The stream was written at a different major version than the registry's.
    IncompatibleVersion,
}

/// A stream that was stepped over. Skipping is a report, not an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The capability the frame declared.
    pub capability: CapabilityId,
    /// The version the frame declared.
    pub version: Version,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// The result of reading a stream set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Streams the reader knows and implements, in file order.
    pub streams: Vec<Stream>,
    /// Streams it stepped over, in file order.
    pub skipped: Vec<Skipped>,
}

fn v1_registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(Registry::v1)
}

fn check_not_geometry(registry: &Registry, id: CapabilityId) -> Result<(), StreamError> {
    match registry.get(id).map(|c| c.class) {
        Some(Class::Geometry) => Err(StreamError::GeometryCapability(id)),
        Some(Class::Component) | None => Ok(()),
        Some(_) => Err(StreamError::NotAComponent(id)),
    }
}

/// Writes a stream set. A stream that declares a geometry capability id of the v1 registry is refused,
/// and so is one that is not a component. An id the registry does not know is written as it is: a tool
/// that passes a newer file's streams through must not lose them.
pub fn encode_streams(streams: &[Stream]) -> Result<Vec<u8>, StreamError> {
    if streams.len() > MAX_STREAMS {
        return Err(StreamError::TooMany);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&SET_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(streams.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for s in streams {
        check_not_geometry(v1_registry(), s.capability)?;
        let len = u32::try_from(s.payload.len()).map_err(|_| StreamError::TooLarge)?;
        out.extend_from_slice(&s.capability.get().to_le_bytes());
        out.extend_from_slice(&s.version.major.to_le_bytes());
        out.extend_from_slice(&s.version.minor.to_le_bytes());
        out.extend_from_slice(&s.version.patch.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&s.payload);
        while out.len() % 8 != 0 {
            out.push(0);
        }
    }
    Ok(out)
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StreamError> {
        if n > self.buf.len() - self.pos {
            return Err(StreamError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, StreamError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, StreamError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Reads a stream set.
///
/// Every frame is stepped over by its declared length, bounds-checked against the bytes present. The
/// payload of a stream that is unknown, not implemented or from another major version is never read.
/// A frame that declares a geometry capability id refuses the whole set.
pub fn read_streams(bytes: &[u8], registry: &Registry, reader: &Reader) -> Result<Opened, StreamError> {
    let mut c = Cursor { buf: bytes, pos: 0 };
    if c.take(4)? != MAGIC {
        return Err(StreamError::BadMagic);
    }
    let version = c.u16()?;
    if version != SET_VERSION {
        return Err(StreamError::UnsupportedVersion(version));
    }
    if c.u16()? != 0 {
        return Err(StreamError::ReservedNotZero);
    }
    let count = c.u32()? as usize;
    if c.u32()? != 0 {
        return Err(StreamError::ReservedNotZero);
    }
    debug_assert_eq!(c.pos, SET_HEADER);
    if count > MAX_STREAMS {
        return Err(StreamError::TooMany);
    }
    // Each frame is at least a header, so the count cannot exceed what the bytes can hold.
    if count > (bytes.len() - SET_HEADER) / FRAME_HEADER {
        return Err(StreamError::Truncated);
    }
    let mut opened = Opened { streams: Vec::new(), skipped: Vec::new() };
    for _ in 0..count {
        let capability = CapabilityId::new(c.u32()?);
        let version = Version::new(c.u16()?, c.u16()?, c.u16()?);
        if c.u16()? != 0 {
            return Err(StreamError::ReservedNotZero);
        }
        let len = c.u32()? as usize;
        let payload = c.take(len)?;
        let pad = (8 - c.pos % 8) % 8;
        if c.take(pad)?.iter().any(|b| *b != 0) {
            return Err(StreamError::NonZeroPadding);
        }
        check_not_geometry(registry, capability)?;
        let skip = |reason| Skipped { capability, version, reason };
        match registry.get(capability) {
            None => opened.skipped.push(skip(SkipReason::UnknownCapability)),
            Some(entry) => {
                if !reader.implemented.contains(&capability) {
                    opened.skipped.push(skip(SkipReason::NotImplemented));
                } else if version.major != entry.version.major {
                    opened.skipped.push(skip(SkipReason::IncompatibleVersion));
                } else {
                    opened.streams.push(Stream { capability, version, payload: payload.to_vec() });
                }
            }
        }
    }
    if c.pos != bytes.len() {
        return Err(StreamError::TrailingBytes);
    }
    Ok(opened)
}
