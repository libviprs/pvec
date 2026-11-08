//! The calibration block (issue 266, ADR 0019): the scale and the georeference of the whole artifact, in one block
//! of kind 11 that capability 22 owns.
//!
//! The block is read whole at open and is small, so it is parsed here next to the other read-at-open payloads and
//! not in the container. Nothing in it is applied to a coordinate: the scale is a claim with its provenance and
//! the georeference is an authored affine with the points it came from, both handed to the host as they are.
//! PVec never interprets the CRS id or the definition text. The byte layout is `docs/CALIBRATION.md` section 1.
//!
//! The rules refuse the calibration and never the file: [`CalibrationError`] carries the `CAL-NN` rule or
//! LIMIT-17, and a reader that meets one drops the calibration and keeps the linework.

use pvec_capabilities::CapabilityId;
use pvec_numeric::detmath::hypot;
use pvec_numeric::{Policy, check_value};
use std::fmt;

/// The registry capability of the calibration block: the next dense id after `raster` (21).
pub const CALIBRATION_CAPABILITY: CapabilityId = CapabilityId::new(22);

/// The block kind capability 22 owns (`docs/CONTAINER.md` chapter 4).
pub const CALIBRATION_KIND: u16 = 11;

/// LIMIT-17: the most bytes the block may be stored as or decode to. `Limits::max_calibration_bytes` of
/// `pvec-container` takes its default from this; the largest block the rules allow is 49,894 bytes.
pub const MAX_CALIBRATION_BYTES: u32 = 65_536;

/// The most control points one georeference may carry (CAL-03).
pub const MAX_CALIBRATION_POINTS: usize = 1_024;

/// `PVCL`, the first four bytes of the block.
const MAGIC: [u8; 4] = *b"PVCL";
/// The bytes of the header and of the two fixed records.
const HEADER: usize = 16;
const SCALE: usize = 80;
const GEO: usize = 72;
/// The longest CRS id, definition, note and external id, in bytes (CAL-03).
const MAX_CRS_ID: usize = 64;
const MAX_DEFINITION: usize = 16_384;
const MAX_STRING: usize = 255;

/// A calibration: a scale, a georeference or both, and the id an application keeps for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    /// What one source unit stands for in the world.
    pub scale: Option<ScaleClaim>,
    /// The map from source space into a CRS.
    pub georeference: Option<Georeference>,
    /// Plain text of at most 255 bytes, empty for none: where the libviprs sidecar's `calibration_id` goes.
    pub external_id: String,
}

impl Calibration {
    /// The number of records present, which is the `count` of the block's directory entry.
    pub fn records(&self) -> u32 {
        u32::from(self.scale.is_some()) + u32::from(self.georeference.is_some())
    }
}

/// The scale record: a claim about the world, never applied to a coordinate.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleClaim {
    /// What one source unit stands for, in metres, in `[2^-40, 2^40]`.
    pub metres_per_source_unit: f64,
    /// Where the claim came from, with its evidence.
    pub provenance: Provenance,
    /// The relative uncertainty, in `[0, 1)`, when one was stated.
    pub uncertainty: Option<f64>,
    /// Plain text of at most 255 bytes, such as the `/R` string of a PDF `/Measure`.
    pub note: String,
}

/// Where a scale came from, and the evidence that provenance carries.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Provenance {
    /// The source declares a unit the manifest table cannot name, such as PDF points.
    DeclaredUnit,
    /// A ratio stated on a sheet, `numerator:denominator`, and what a source unit is on the paper.
    PlanScale {
        /// The `1` of `1:100`.
        numerator: u32,
        /// The `100` of `1:100`.
        denominator: u32,
        /// Metres per source unit on the paper, in `[2^-40, 2^40]`.
        paper_metres_per_unit: f64,
    },
    /// Two source points and the real distance between them.
    Measured {
        /// The first source point.
        p0: [f64; 2],
        /// The second source point.
        p1: [f64; 2],
        /// The real distance between them, in metres.
        distance_metres: f64,
    },
    /// Derived from the georeference record of the same block.
    Georeference,
    /// Typed in by somebody.
    Manual,
}

impl Provenance {
    /// The wire byte.
    pub fn word(&self) -> u8 {
        match self {
            Provenance::DeclaredUnit => 1,
            Provenance::PlanScale { .. } => 2,
            Provenance::Measured { .. } => 3,
            Provenance::Georeference => 4,
            Provenance::Manual => 5,
        }
    }
}

/// The georeference record: an authored affine from source space into a CRS, and its evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct Georeference {
    /// `[a b c d tx ty]`: `X = a*x + b*y + tx`, `Y = c*x + d*y + ty`, held to NP-19. The output is (easting,
    /// northing), or (longitude, latitude) for a geographic CRS.
    pub source_to_crs: [f64; 6],
    /// What kind of CRS the output is in.
    pub crs_kind: CrsKind,
    /// How the affine was arrived at.
    pub method: FitMethod,
    /// An authority code, `AUTHORITY:CODE`, at most 64 bytes of `[A-Za-z0-9:_.+-]`, empty for none.
    pub crs_id: String,
    /// The CRS definition, opaque to every PVec crate.
    pub definition: Option<CrsDefinition>,
    /// The evidence, at most 1,024 points.
    pub control_points: Vec<ControlPoint>,
    /// The fit's largest residual in CRS units; zero unless `method` is `Fitted` with at least 3 points.
    pub residual_max: f64,
}

/// The kind of CRS a georeference maps into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CrsKind {
    /// A projected CRS: easting and northing.
    Projected,
    /// A geographic CRS: longitude and latitude.
    Geographic,
    /// An engineering or local CRS, such as a site grid.
    Engineering,
}

/// How a georeference's affine was arrived at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FitMethod {
    /// Declared by the source.
    Declared,
    /// Fitted by the writer from the control points: a similarity from 2, an affine from 3 or more.
    Fitted,
    /// Typed in by somebody.
    Manual,
}

/// A CRS definition: opaque text tagged with its format, handed to the host as bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct CrsDefinition {
    /// What the text is written in.
    pub format: DefinitionFormat,
    /// At most 16,384 bytes of plain text; LF and TAB are the only controls allowed.
    pub text: String,
}

/// The format of a CRS definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DefinitionFormat {
    /// OGC WKT2.
    Wkt2,
    /// PROJJSON.
    Projjson,
}

/// One control point: a source point and the world point it stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPoint {
    /// In source space.
    pub source: [f64; 2],
    /// In the CRS, in the axis order of [`Georeference::source_to_crs`].
    pub world: [f64; 2],
}

/// Why a calibration was refused. The calibration is, and the file is not.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CalibrationError {
    /// LIMIT-17: the block is larger than the ceiling.
    TooLarge {
        /// The block's length.
        declared: u64,
    },
    /// CAL-01: the magic, the version, a flag bit or a reserved field.
    BadHeader,
    /// CAL-02: the length field and the bytes disagree, or the bytes are not consumed exactly.
    BadLength,
    /// CAL-03: a count or a length over its cap.
    TooMany,
    /// CAL-04: a number that is not finite, is subnormal or `-0.0`, or is out of its range.
    BadNumber,
    /// CAL-05: the affine fails NP-19.
    BadAffine,
    /// CAL-06: contradictory control points, or a degenerate measured pair.
    BadPoints,
    /// CAL-07: text that is not plain, or a CRS id outside its character class.
    BadText,
    /// CAL-08: an undefined tag.
    BadTag,
    /// CAL-09: no record, or a georeference with neither a CRS id nor a definition.
    Empty,
    /// CAL-10: evidence that does not fit its provenance or method.
    BadEvidence,
    /// CAL-11 (`pvec validate` only): the residual recomputed from the points is not the stored one.
    ResidualMismatch {
        /// What the block stores.
        stored: f64,
        /// What the points and the affine give.
        recomputed: f64,
    },
    /// CAL-12: a second kind 11 block, or a kind 11 entry with `cell`, `subtype` or a flag set.
    Directory,
}

impl CalibrationError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            CalibrationError::TooLarge { .. } => "LIMIT-17",
            CalibrationError::BadHeader => "CAL-01",
            CalibrationError::BadLength => "CAL-02",
            CalibrationError::TooMany => "CAL-03",
            CalibrationError::BadNumber => "CAL-04",
            CalibrationError::BadAffine => "CAL-05",
            CalibrationError::BadPoints => "CAL-06",
            CalibrationError::BadText => "CAL-07",
            CalibrationError::BadTag => "CAL-08",
            CalibrationError::Empty => "CAL-09",
            CalibrationError::BadEvidence => "CAL-10",
            CalibrationError::ResidualMismatch { .. } => "CAL-11",
            CalibrationError::Directory => "CAL-12",
        }
    }
}

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.rule();
        match self {
            CalibrationError::TooLarge { declared } => {
                write!(
                    f,
                    "the calibration block is {declared} bytes against the format's ceiling of {MAX_CALIBRATION_BYTES} ({r})"
                )
            }
            CalibrationError::ResidualMismatch { stored, recomputed } => write!(
                f,
                "the calibration stores a residual of {stored} and its points give {recomputed} ({r})"
            ),
            _ => {
                let meaning =
                    CALIBRATION_ERRORS.iter().find(|(rule, _, _)| *rule == r).map_or("", |(_, _, m)| *m);
                write!(f, "the calibration is refused: {meaning} ({r})")
            }
        }
    }
}

impl std::error::Error for CalibrationError {}

/// The rows of the rule table of `docs/CALIBRATION.md`: rule, error and meaning. A test renders the table from
/// this and fails if the document differs.
pub const CALIBRATION_ERRORS: &[(&str, &str, &str)] = &[
    (
        "LIMIT-17",
        "TooLarge",
        "the block is stored as, or decodes to, more than `max_calibration_bytes` (65,536 by default and in the browser preset), refused from the directory entry before a byte is fetched",
    ),
    (
        "CAL-01",
        "BadHeader",
        "the magic is not `PVCL`, the version is not 1, a flag bit other than 0 and 1 is set, or a reserved field is not zero",
    ),
    (
        "CAL-02",
        "BadLength",
        "the `length` field is not the payload's length, the structure ends before the bytes do, or bytes are left over",
    ),
    (
        "CAL-03",
        "TooMany",
        "more than 1,024 control points, a CRS id over 64 bytes, a definition over 16,384 bytes, or a note or external id over 255 bytes (which only a writer can be handed)",
    ),
    (
        "CAL-04",
        "BadNumber",
        "a float that is not finite, is subnormal, is `-0.0` or passes 2^40 (NP-01, NP-02); a factor or a distance outside `[2^-40, 2^40]`; an uncertainty outside `[0, 1)`; a negative residual",
    ),
    (
        "CAL-05",
        "BadAffine",
        "the affine fails NP-19: singular under PVec's rule, or a linear entry above 2^20",
    ),
    (
        "CAL-06",
        "BadPoints",
        "two control points with the same source point and different world points, or a measured pair no further apart than NP-06's degenerate length",
    ),
    (
        "CAL-07",
        "BadText",
        "a CRS id outside `[A-Za-z0-9:_.+-]`, or a definition, note or external id that is not UTF-8 or holds a control (LF and TAB in a definition aside), a line or paragraph separator or a bidi override",
    ),
    (
        "CAL-08",
        "BadTag",
        "an undefined provenance, method, CRS kind or definition format, a `has_uncertainty` other than 0 or 1, or a definition format that disagrees with the definition's length",
    ),
    ("CAL-09", "Empty", "neither record is present, or a georeference has neither a CRS id nor a definition"),
    (
        "CAL-10",
        "BadEvidence",
        "evidence set for a provenance that does not use it or missing for one that does, an uncertainty without its flag, a residual on a record that is not fitted from at least 3 points, a fitted record with fewer than 2 points, or provenance `georeference` without a georeference record",
    ),
    (
        "CAL-11",
        "ResidualMismatch",
        "`pvec validate` only: the residual recomputed from the points and the affine is not NP-05 near the stored one at the scale of the largest world coordinate",
    ),
    (
        "CAL-12",
        "Directory",
        "a second kind 11 block, or a kind 11 entry with `cell`, `subtype` or a flag set",
    ),
];

/// NP-01, NP-02 and NP-03 (CAL-04). A `-0.0` passes here and is written as `+0.0`; a stored `-0.0` is refused by
/// the decoder, since the stored bits are the canonical ones.
fn number(x: f64) -> Result<f64, CalibrationError> {
    check_value(x).map_err(|_| CalibrationError::BadNumber)
}

/// A factor or a distance: a number in `[2^-40, 2^40]` (CAL-04).
fn factor(x: f64) -> Result<f64, CalibrationError> {
    let (min, max) = (1.0 / (1u64 << 40) as f64, (1u64 << 40) as f64);
    if (min..=max).contains(&number(x)?) { Ok(x) } else { Err(CalibrationError::BadNumber) }
}

/// The RASTER-07 text rules: no control (LF and TAB aside when `lf_tab`), no line or paragraph separator, no
/// bidi override or isolate (CAL-07).
fn plain(text: &str, lf_tab: bool) -> Result<(), CalibrationError> {
    let bad = |ch: char| {
        (ch.is_control() && !(lf_tab && matches!(ch, '\n' | '\t')))
            || matches!(ch, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    };
    if text.chars().any(bad) { Err(CalibrationError::BadText) } else { Ok(()) }
}

/// The rules that need no bytes: what both the encoder and the decoder refuse.
fn check(c: &Calibration) -> Result<(), CalibrationError> {
    use CalibrationError as E;
    if c.scale.is_none() && c.georeference.is_none() {
        return Err(E::Empty);
    }
    if c.external_id.len() > MAX_STRING {
        return Err(E::TooMany);
    }
    plain(&c.external_id, false)?;
    if let Some(s) = &c.scale {
        if s.note.len() > MAX_STRING {
            return Err(E::TooMany);
        }
        factor(s.metres_per_source_unit)?;
        if let Some(u) = s.uncertainty {
            if !(0.0..1.0).contains(&number(u)?) {
                return Err(E::BadNumber);
            }
        }
        match s.provenance {
            Provenance::PlanScale { numerator, denominator, paper_metres_per_unit } => {
                factor(paper_metres_per_unit)?;
                if numerator == 0 || denominator == 0 {
                    return Err(E::BadEvidence);
                }
            }
            Provenance::Measured { p0, p1, distance_metres } => {
                for x in [p0[0], p0[1], p1[0], p1[1]] {
                    number(x)?;
                }
                factor(distance_metres)?;
                let mag = [p0[0], p0[1], p1[0], p1[1]].iter().fold(0.0f64, |m, x| m.max(x.abs()));
                if Policy::V1.degenerate_length(hypot(p1[0] - p0[0], p1[1] - p0[1]), mag) {
                    return Err(E::BadPoints);
                }
            }
            Provenance::Georeference if c.georeference.is_none() => return Err(E::BadEvidence),
            _ => {}
        }
        plain(&s.note, false)?;
    }
    if let Some(g) = &c.georeference {
        if g.control_points.len() > MAX_CALIBRATION_POINTS
            || g.crs_id.len() > MAX_CRS_ID
            || g.definition.as_ref().is_some_and(|d| d.text.len() > MAX_DEFINITION)
        {
            return Err(E::TooMany);
        }
        for x in g.source_to_crs {
            number(x)?;
        }
        Policy::V1.validate_affine(g.source_to_crs).map_err(|_| E::BadAffine)?;
        if number(g.residual_max)? < 0.0 {
            return Err(E::BadNumber);
        }
        for p in &g.control_points {
            for x in [p.source[0], p.source[1], p.world[0], p.world[1]] {
                number(x)?;
            }
        }
        let n = g.control_points.len();
        if (g.method == FitMethod::Fitted && n < 2)
            || (g.residual_max != 0.0 && !(g.method == FitMethod::Fitted && n >= 3))
        {
            return Err(E::BadEvidence);
        }
        // a source point given two world points: sorted by the bits as they are stored, `x + 0.0` turning a -0.0
        // into the +0.0 the encoder writes, so two points the block cannot tell apart compare equal here (NaN is
        // refused above)
        let bits = |x: f64| (x + 0.0).to_bits();
        let mut keyed: Vec<[u64; 4]> = g
            .control_points
            .iter()
            .map(|p| [bits(p.source[0]), bits(p.source[1]), bits(p.world[0]), bits(p.world[1])])
            .collect();
        keyed.sort_unstable();
        if keyed.windows(2).any(|w| w[0][..2] == w[1][..2] && w[0] != w[1]) {
            return Err(E::BadPoints);
        }
        if !g.crs_id.bytes().all(|b| b.is_ascii_alphanumeric() || b":_.+-".contains(&b)) {
            return Err(E::BadText);
        }
        match &g.definition {
            Some(d) if d.text.is_empty() => return Err(E::BadTag),
            Some(d) => plain(&d.text, true)?,
            None if g.crs_id.is_empty() => return Err(E::Empty),
            None => {}
        }
    }
    Ok(())
}

/// The block for `c`. Refuses what a reader would refuse (everything but CAL-11, which is `pvec validate`'s), and
/// writes the evidence a provenance does not use as zero.
pub fn encode_calibration(c: &Calibration) -> Result<Vec<u8>, CalibrationError> {
    check(c)?;
    let mut out = Vec::with_capacity(HEADER + SCALE + GEO);
    // `x + 0.0` is `x`, except that `-0.0` becomes `+0.0` (NP-03), so a zero has one byte form
    let f = |out: &mut Vec<u8>, x: f64| out.extend_from_slice(&(x + 0.0).to_le_bytes());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&1u16.to_le_bytes());
    let flags = u16::from(c.scale.is_some()) | (u16::from(c.georeference.is_some()) << 1);
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // the length, filled in at the end
    out.push(c.external_id.len() as u8);
    out.extend_from_slice(&[0; 3]);
    if let Some(s) = &c.scale {
        f(&mut out, s.metres_per_source_unit);
        f(&mut out, s.uncertainty.unwrap_or(0.0));
        let (paper, numerator, denominator) = match s.provenance {
            Provenance::PlanScale { numerator, denominator, paper_metres_per_unit } => {
                (paper_metres_per_unit, numerator, denominator)
            }
            _ => (0.0, 0, 0),
        };
        f(&mut out, paper);
        out.extend_from_slice(&numerator.to_le_bytes());
        out.extend_from_slice(&denominator.to_le_bytes());
        let (p0, p1, distance) = match s.provenance {
            Provenance::Measured { p0, p1, distance_metres } => (p0, p1, distance_metres),
            _ => ([0.0; 2], [0.0; 2], 0.0),
        };
        for x in [p0[0], p0[1], p1[0], p1[1], distance] {
            f(&mut out, x);
        }
        out.extend_from_slice(&[s.provenance.word(), u8::from(s.uncertainty.is_some()), s.note.len() as u8]);
        out.extend_from_slice(&[0; 5]);
    }
    if let Some(g) = &c.georeference {
        for x in g.source_to_crs {
            f(&mut out, x);
        }
        f(&mut out, g.residual_max);
        let definition = g.definition.as_ref();
        out.extend_from_slice(&(definition.map_or(0, |d| d.text.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(g.control_points.len() as u16).to_le_bytes());
        let crs_kind = match g.crs_kind {
            CrsKind::Projected => 1,
            CrsKind::Geographic => 2,
            CrsKind::Engineering => 3,
        };
        let method = match g.method {
            FitMethod::Declared => 1,
            FitMethod::Fitted => 2,
            FitMethod::Manual => 3,
        };
        let format = match definition.map(|d| d.format) {
            None => 0,
            Some(DefinitionFormat::Wkt2) => 1,
            Some(DefinitionFormat::Projjson) => 2,
        };
        out.extend_from_slice(&[crs_kind, method, g.crs_id.len() as u8, format]);
        out.extend_from_slice(&[0; 6]);
        for p in &g.control_points {
            for x in [p.source[0], p.source[1], p.world[0], p.world[1]] {
                f(&mut out, x);
            }
        }
        out.extend_from_slice(g.crs_id.as_bytes());
        if let Some(d) = definition {
            out.extend_from_slice(d.text.as_bytes());
        }
    }
    if let Some(s) = &c.scale {
        out.extend_from_slice(s.note.as_bytes());
    }
    out.extend_from_slice(c.external_id.as_bytes());
    let n = out.len() as u32;
    out[8..12].copy_from_slice(&n.to_le_bytes());
    Ok(out)
}

/// Reads a block, which must be consumed exactly. LIMIT-17 holds the length before anything is looked at, the
/// counts in the fixed records are held to their caps before the tail is sized from them, and the result is held
/// to every rule but CAL-11.
pub fn decode_calibration(bytes: &[u8]) -> Result<Calibration, CalibrationError> {
    use CalibrationError as E;
    if bytes.len() > MAX_CALIBRATION_BYTES as usize {
        return Err(E::TooLarge { declared: bytes.len() as u64 });
    }
    if bytes.len() < HEADER {
        return Err(E::BadLength);
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let f64_at = |at: usize| f64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"));
    let flags = u16_at(6);
    if bytes[0..4] != MAGIC || u16_at(4) != 1 || flags & !3 != 0 || bytes[13..16] != [0, 0, 0] {
        return Err(E::BadHeader);
    }
    if u32_at(8) as usize != bytes.len() {
        return Err(E::BadLength);
    }
    if flags == 0 {
        return Err(E::Empty);
    }
    let (has_scale, has_geo) = (flags & 1 != 0, flags & 2 != 0);
    let fixed = HEADER + if has_scale { SCALE } else { 0 } + if has_geo { GEO } else { 0 };
    if bytes.len() < fixed {
        return Err(E::BadLength);
    }
    let s = HEADER;
    let g = if has_scale { HEADER + SCALE } else { HEADER };
    if (has_scale && bytes[s + 75..s + 80] != [0; 5]) || (has_geo && bytes[g + 66..g + 72] != [0; 6]) {
        return Err(E::BadHeader);
    }
    // every float of the fixed records, unused evidence included, is held to NP-01 to NP-03 first, and a stored
    // `-0.0` is refused: the writer never stores one, so a block that parses is the block that encodes
    let floats = |at: usize, n: usize| {
        (0..n).try_for_each(|i| match f64_at(at + 8 * i) {
            x if x.to_bits() == (-0.0f64).to_bits() => Err(E::BadNumber),
            x => number(x).map(drop),
        })
    };
    if has_scale {
        floats(s, 3)?;
        floats(s + 32, 5)?;
    }
    if has_geo {
        floats(g, 7)?;
    }
    let (points, id_len, def_len) = if has_geo {
        (usize::from(u16_at(g + 60)), usize::from(bytes[g + 64]), u32_at(g + 56) as usize)
    } else {
        (0, 0, 0)
    };
    if points > MAX_CALIBRATION_POINTS || id_len > MAX_CRS_ID || def_len > MAX_DEFINITION {
        return Err(E::TooMany);
    }
    let note_len = if has_scale { usize::from(bytes[s + 74]) } else { 0 };
    let ext_len = usize::from(bytes[12]);
    if fixed + points * 32 + id_len + def_len + note_len + ext_len != bytes.len() {
        return Err(E::BadLength);
    }
    floats(fixed, points * 4)?;
    let mut at = fixed;
    let mut take = |n: usize| {
        let t = &bytes[at..at + n];
        at += n;
        t
    };
    let control_points: Vec<ControlPoint> = (0..points)
        .map(|_| {
            let p = take(32);
            let x = |k: usize| f64::from_le_bytes(p[8 * k..8 * k + 8].try_into().expect("eight bytes"));
            ControlPoint { source: [x(0), x(1)], world: [x(2), x(3)] }
        })
        .collect();
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|_| E::BadText);
    let crs_id = text(take(id_len))?;
    let definition_text = text(take(def_len))?;
    let note = text(take(note_len))?;
    let external_id = text(take(ext_len))?;
    let scale = if has_scale {
        let (paper, numerator, denominator) = (f64_at(s + 16), u32_at(s + 24), u32_at(s + 28));
        let (p0, p1, distance) =
            ([f64_at(s + 32), f64_at(s + 40)], [f64_at(s + 48), f64_at(s + 56)], f64_at(s + 64));
        let plan_unused = paper.to_bits() == 0 && numerator == 0 && denominator == 0;
        let measured_unused = [p0[0], p0[1], p1[0], p1[1], distance].iter().all(|x| x.to_bits() == 0);
        let provenance = match bytes[s + 72] {
            1 => Provenance::DeclaredUnit,
            2 => Provenance::PlanScale { numerator, denominator, paper_metres_per_unit: paper },
            3 => Provenance::Measured { p0, p1, distance_metres: distance },
            4 => Provenance::Georeference,
            5 => Provenance::Manual,
            _ => return Err(E::BadTag),
        };
        let unused_clear = match provenance {
            Provenance::PlanScale { .. } => measured_unused,
            Provenance::Measured { .. } => plan_unused,
            _ => plan_unused && measured_unused,
        };
        let uncertainty = match bytes[s + 73] {
            0 if f64_at(s + 8).to_bits() == 0 => None,
            0 => return Err(E::BadEvidence),
            1 => Some(f64_at(s + 8)),
            _ => return Err(E::BadTag),
        };
        if !unused_clear {
            return Err(E::BadEvidence);
        }
        Some(ScaleClaim { metres_per_source_unit: f64_at(s), provenance, uncertainty, note })
    } else {
        None
    };
    let georeference = if has_geo {
        let crs_kind = match bytes[g + 62] {
            1 => CrsKind::Projected,
            2 => CrsKind::Geographic,
            3 => CrsKind::Engineering,
            _ => return Err(E::BadTag),
        };
        let method = match bytes[g + 63] {
            1 => FitMethod::Declared,
            2 => FitMethod::Fitted,
            3 => FitMethod::Manual,
            _ => return Err(E::BadTag),
        };
        let definition = match (bytes[g + 65], def_len) {
            (0, 0) => None,
            (1, 1..) => Some(CrsDefinition { format: DefinitionFormat::Wkt2, text: definition_text }),
            (2, 1..) => Some(CrsDefinition { format: DefinitionFormat::Projjson, text: definition_text }),
            _ => return Err(E::BadTag),
        };
        let mut source_to_crs = [0.0; 6];
        for (i, x) in source_to_crs.iter_mut().enumerate() {
            *x = f64_at(g + 8 * i);
        }
        Some(Georeference {
            source_to_crs,
            crs_kind,
            method,
            crs_id,
            definition,
            control_points,
            residual_max: f64_at(g + 48),
        })
    } else {
        None
    };
    let c = Calibration { scale, georeference, external_id };
    check(&c)?;
    Ok(c)
}

/// CAL-11, `pvec validate`'s check of the producer's arithmetic: the largest residual of the points under the
/// affine, recomputed with `+`, `-`, `*` and [`hypot`] only, left to right, so every platform gets the same bits,
/// and compared with the stored one by NP-05 at the scale of the largest world coordinate. Only a record fitted
/// from at least 3 points has a residual to check; for any other the answer is zero.
pub fn check_residual(g: &Georeference) -> Result<f64, CalibrationError> {
    if !(g.method == FitMethod::Fitted && g.control_points.len() >= 3) {
        return Ok(0.0);
    }
    let [a, b, c, d, tx, ty] = g.source_to_crs;
    let (mut recomputed, mut big) = (0.0f64, 0.0f64);
    for p in &g.control_points {
        let [sx, sy] = p.source;
        let [wx, wy] = p.world;
        let ex = a * sx + b * sy + tx - wx;
        let ey = c * sx + d * sy + ty - wy;
        recomputed = recomputed.max(hypot(ex, ey));
        big = big.max(wx.abs()).max(wy.abs());
    }
    if Policy::V1.near(recomputed, g.residual_max, big) {
        Ok(recomputed)
    } else {
        Err(CalibrationError::ResidualMismatch { stored: g.residual_max, recomputed })
    }
}
