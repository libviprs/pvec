//! The geometry codecs (issues P3.2 and P3.3): `pvec-geometry` curves to the block payload that
//! ADR 0004 chose, and back.
//!
//! A block groups its entities by section kind (type-batched, structure of arrays): one count and a
//! set of columns per kind present, in ascending kind order. Fixed-width kinds are plain `f64`
//! columns; polylines and composites carry an offset table into an arena. The byte layout is in
//! `docs/GEOMETRY_ENCODING.md`; the block framing, compression and checksums around it are phase 4.
//!
//! # Guarantees
//!
//! * Bit-exact: the numbers that go in are the numbers that come out, including trim parameters in
//!   their normalised form. There is no `f32` anywhere on the path.
//! * Kind-preserving: an arc is a trimmed circle on the way out as it was on the way in, and a
//!   polyline is never promoted. A curve kind with no codec is refused ([`EncodeError::Unsupported`]),
//!   never encoded as something else.
//! * Canonical: the same entities in the same order always give the same bytes, and decoding then
//!   encoding reproduces them.
//! * Validated: the decoder rebuilds every curve through the constructors of `pvec-geometry`, so a
//!   malformed block cannot yield a curve that was not validated. Every count is compared with the
//!   bytes really present before it can size an allocation.
//! * No wire or serialization-generated type appears in a public signature. The public API is
//!   snapshotted in `api.txt` and checked by `pvec-checks`.

mod decode;
mod encode;
mod error;
mod records;
mod rows;
mod section;
mod wire;

use pvec_capabilities::{CapabilityId, ids};
use pvec_geometry::{Curve, Curve2d, CurveKind};

pub use decode::{decode, describe};
pub use encode::{Encoded, encode};
pub use error::{DecodeError, EncodeError};
pub use section::Section;

/// The first four bytes of every block.
pub(crate) const MAGIC: [u8; 4] = *b"PVG1";

/// The layout version written and the only one read.
pub const FORMAT_VERSION: u16 = 1;

/// The registry capability a curve kind needs. The ids come from `pvec-capabilities`, not from this
/// crate. `None` for a kind the registry does not know (the kind enum may grow).
pub fn capability_for(kind: CurveKind) -> Option<CapabilityId> {
    match kind {
        CurveKind::Line => Some(ids::LINE),
        CurveKind::Circle => Some(ids::CIRCLE),
        CurveKind::Ellipse => Some(ids::ELLIPSE),
        CurveKind::Polyline => Some(ids::POLYLINE),
        CurveKind::TrimmedCurve => Some(ids::TRIMMED_CURVE),
        CurveKind::CompositeCurve => Some(ids::COMPOSITE_CURVE),
        CurveKind::Bezier => Some(ids::BEZIER_CURVE),
        CurveKind::BSpline => Some(ids::B_SPLINE_CURVE),
        CurveKind::RationalBSpline => Some(ids::RATIONAL_B_SPLINE_CURVE),
        _ => None,
    }
}

/// Every capability a curve needs: its own kind, plus the basis of a trimmed curve and the segments
/// of a composite, recursively.
pub fn capabilities_of(curve: &Curve) -> Vec<CapabilityId> {
    let mut out: Vec<CapabilityId> = capability_for(curve.kind()).into_iter().collect();
    match curve {
        Curve::Trimmed(t) => out.extend(capabilities_of(t.basis())),
        Curve::Composite(c) => {
            for s in c.segments() {
                out.extend(capabilities_of(s));
            }
        }
        _ => {}
    }
    out.sort();
    out.dedup();
    out
}
