//! The fixed-width rows: how a line, circle, ellipse or trimmed arc becomes a handful of `f64` and
//! back. The same rows are used by the standalone sections and by the segments of a composite.

use crate::section::Section;
use pvec_geometry::{
    Circle, Curve, Ellipse, GeometryError, Line, Placement2, Point2, Trim, Trimmed, Vector2,
};

/// A curve as a section kind, up to eight numbers and the `reversed` flag.
pub(crate) struct FixedRow {
    pub(crate) section: Section,
    pub(crate) vals: [f64; 8],
    pub(crate) n: usize,
    pub(crate) reversed: bool,
}

fn row(section: Section, v: &[f64], reversed: bool) -> FixedRow {
    let mut vals = [0.0; 8];
    vals[..v.len()].copy_from_slice(v);
    FixedRow { section, vals, n: v.len(), reversed }
}

fn line_vals(l: &Line) -> [f64; 4] {
    [l.origin().x, l.origin().y, l.direction().x, l.direction().y]
}

fn frame_vals(p: &Placement2) -> [f64; 4] {
    [p.origin().x, p.origin().y, p.x_axis().x, p.x_axis().y]
}

/// The fixed row of a curve, or `None` when the curve is not one of the six fixed kinds.
pub(crate) fn fixed_row(c: &Curve) -> Option<FixedRow> {
    match c {
        Curve::Line(l) => Some(row(Section::Line, &line_vals(l), false)),
        Curve::Circle(c) => {
            let f = frame_vals(c.placement());
            Some(row(Section::Circle, &[f[0], f[1], f[2], f[3], c.radius()], false))
        }
        Curve::Ellipse(e) => {
            let f = frame_vals(e.placement());
            Some(row(Section::Ellipse, &[f[0], f[1], f[2], f[3], e.semi_axis_1(), e.semi_axis_2()], false))
        }
        Curve::Trimmed(t) => {
            let Trim { start, span, reversed } = t.trim();
            match t.basis() {
                Curve::Line(l) => {
                    let v = line_vals(l);
                    Some(row(Section::TrimmedLine, &[v[0], v[1], v[2], v[3], start, span], reversed))
                }
                Curve::Circle(c) => {
                    let f = frame_vals(c.placement());
                    Some(row(
                        Section::TrimmedCircle,
                        &[f[0], f[1], f[2], f[3], c.radius(), start, span],
                        reversed,
                    ))
                }
                Curve::Ellipse(e) => {
                    let f = frame_vals(e.placement());
                    Some(row(
                        Section::TrimmedEllipse,
                        &[f[0], f[1], f[2], f[3], e.semi_axis_1(), e.semi_axis_2(), start, span],
                        reversed,
                    ))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Rebuilds a curve from a fixed row through the validating constructors. `vals` must hold the
/// number of values `section` needs; the caller reads exactly that many.
pub(crate) fn build_fixed(section: Section, vals: &[f64], reversed: bool) -> Result<Curve, GeometryError> {
    let frame = || Placement2::from_stored(Point2::new(vals[0], vals[1]), Vector2::new(vals[2], vals[3]));
    let trim = |at: usize| Trim { start: vals[at], span: vals[at + 1], reversed };
    match section {
        Section::Line => {
            Ok(Curve::Line(Line::from_stored(Point2::new(vals[0], vals[1]), Vector2::new(vals[2], vals[3]))?))
        }
        Section::Circle => Ok(Curve::Circle(Circle::new(frame()?, vals[4])?)),
        Section::Ellipse => Ok(Curve::Ellipse(Ellipse::new(frame()?, vals[4], vals[5])?)),
        Section::TrimmedLine => {
            let basis = Curve::Line(Line::from_stored(
                Point2::new(vals[0], vals[1]),
                Vector2::new(vals[2], vals[3]),
            )?);
            Ok(Curve::Trimmed(Trimmed::from_normalised(basis, trim(4))?))
        }
        Section::TrimmedCircle => {
            let basis = Curve::Circle(Circle::new(frame()?, vals[4])?);
            Ok(Curve::Trimmed(Trimmed::from_normalised(basis, trim(5))?))
        }
        Section::TrimmedEllipse => {
            let basis = Curve::Ellipse(Ellipse::new(frame()?, vals[4], vals[5])?);
            Ok(Curve::Trimmed(Trimmed::from_normalised(basis, trim(6))?))
        }
        // Not a fixed section: the decoder never asks, and a refusal is safer than a panic if it did.
        _ => Err(GeometryError::BadBasis),
    }
}
