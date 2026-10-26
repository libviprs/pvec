//! The entities of a file as curves of the curve model, for the subcommands that evaluate geometry.
//!
//! The container is not a mirror of the model: a Bezier is stored as a clamped B-spline, a trimmed
//! line as a segment, and the frame of a rotated conic as an angle. This module reads back what the
//! file holds, so a Bezier decodes as a B-spline and a trimmed circle as a trimmed circle in the
//! standard frame. An entity that cannot be decoded is returned with the reason, never dropped.

use core::f64::consts::TAU;
use pvec_container::blocks::advanced::CurveView;
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReadError};
use pvec_container::source::BlockSource;
use pvec_geometry::{
    BSpline, Circle, Composite, Curve, Ellipse, GeometryError, Line, Placement2, Point2, Polyline,
    RationalBSpline, Trimmed, Vector2,
};

enum Slot {
    Pending,
    Done(Curve),
    Failed,
}

/// What decoding found: the curves ascending by entity id, and the entities that could not be decoded.
pub struct Decoded {
    /// `(entity id, curve)`.
    pub curves: Vec<(u32, Curve)>,
    /// `(entity id, reason)`.
    pub unsupported: Vec<(u32, String)>,
}

fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn frame(origin: Point2, angle: f64) -> Result<Placement2, GeometryError> {
    Placement2::new(origin, Vector2::new(angle.cos(), angle.sin()))
}

fn arc(basis: Curve, start: f64, sweep: f64) -> Result<Curve, GeometryError> {
    // a reversed arc is stored as a negative sweep from its start
    Curve::trimmed_conic(basis, start, start + sweep, sweep >= 0.0)
}

/// Decodes every entity of an opened file.
pub fn decode<S: BlockSource>(reader: &ContainerReader<S>) -> Result<Decoded, ReadError> {
    let mut out = Decoded { curves: Vec::new(), unsupported: Vec::new() };
    for (i, e) in reader.entries().iter().enumerate() {
        if e.block_kind() != Some(BlockKind::Hot) {
            continue;
        }
        let block = reader.read_block(BlockId(i as u32))?;
        let v = block.hot()?;
        let col = |n: &str| v.column(n).unwrap_or(&[]);
        for (k, id) in v.entity_ids().iter().enumerate() {
            let curve = match e.subtype {
                1 => {
                    let (a, b) = (pt(col("x0")[k], col("y0")[k]), pt(col("x1")[k], col("y1")[k]));
                    let len = a.distance(b);
                    Line::new(a, Vector2::new((b.x - a.x) / len, (b.y - a.y) / len))
                        .and_then(|l| Trimmed::new(Curve::Line(l), 0.0, len, true).map(Curve::Trimmed))
                }
                2 => v.polyline(k).ok_or(GeometryError::BadSegment).and_then(|(x, y)| {
                    Polyline::new(x.iter().zip(y).map(|(x, y)| pt(*x, *y)).collect()).map(Curve::Polyline)
                }),
                3 => {
                    let (c, r, start, sweep) =
                        (pt(col("cx")[k], col("cy")[k]), col("radius")[k], col("start")[k], col("sweep")[k]);
                    if sweep.abs() == TAU {
                        frame(c, start).and_then(|f| Circle::new(f, r)).map(Curve::Circle)
                    } else {
                        frame(c, 0.0)
                            .and_then(|f| Circle::new(f, r))
                            .and_then(|b| arc(Curve::Circle(b), start, sweep))
                    }
                }
                4 => {
                    let (c, rx, ry) = (pt(col("cx")[k], col("cy")[k]), col("rx")[k], col("ry")[k]);
                    let (rot, start, sweep) = (col("rotation")[k], col("start")[k], col("sweep")[k]);
                    let basis = frame(c, rot).and_then(|f| Ellipse::new(f, rx, ry)).map(Curve::Ellipse);
                    if sweep.abs() == TAU && start == 0.0 {
                        basis
                    } else {
                        basis.and_then(|b| arc(b, start, sweep))
                    }
                }
                _ => {
                    out.unsupported.push((*id, format!("hot kind {}", e.subtype)));
                    continue;
                }
            };
            match curve {
                Ok(c) => out.curves.push((*id, c)),
                Err(e) => out.unsupported.push((*id, e.rule().to_string())),
            }
        }
    }
    for chunk in 0..reader.advanced_chunk_count()? {
        let Some(set) = reader.advanced(chunk)? else { continue };
        let view = set.view()?;
        let mut slots: Vec<Slot> = (0..view.len()).map(|_| Slot::Pending).collect();
        // segments refer to records by index and nest to a checked depth, so passes in order of
        // dependency finish: repeat until a pass settles nothing more
        let mut settled = true;
        while settled {
            settled = false;
            for index in 0..view.len() {
                if !matches!(slots[index], Slot::Pending) {
                    continue;
                }
                let built: Option<Result<Curve, String>> = match view.curve(index)? {
                    CurveView::Spline(s) => {
                        let pts: Vec<Point2> = s.x.iter().zip(s.y).map(|(x, y)| pt(*x, *y)).collect();
                        let closed = |r: Result<Curve, GeometryError>| r.map_err(|e| e.rule().to_string());
                        Some(closed(match s.weights {
                            Some(w) => RationalBSpline::new(s.degree, pts, s.knots.to_vec(), w.to_vec())
                                .and_then(|b| if s.closed { b.declare_closed() } else { Ok(b) })
                                .map(Curve::RationalBSpline),
                            None => BSpline::new(s.degree, pts, s.knots.to_vec())
                                .and_then(|b| if s.closed { b.declare_closed() } else { Ok(b) })
                                .map(Curve::BSpline),
                        }))
                    }
                    CurveView::Composite(c) if c.trims.is_some() => {
                        Some(Err("a composite that trims its segments".to_string()))
                    }
                    CurveView::Composite(c) => {
                        let members: Vec<&Slot> = c
                            .segments
                            .iter()
                            .map(|s| slots.get(*s as usize).unwrap_or(&Slot::Failed))
                            .collect();
                        if members.iter().any(|m| matches!(m, Slot::Failed)) {
                            Some(Err("a segment that could not be decoded".to_string()))
                        } else if members.iter().any(|m| matches!(m, Slot::Pending)) {
                            None
                        } else {
                            let parts: Vec<Curve> = members
                                .iter()
                                .filter_map(|m| if let Slot::Done(c) = m { Some(c.clone()) } else { None })
                                .collect();
                            Some(
                                Composite::new(parts)
                                    .and_then(|k| if c.closed { k.declare_closed() } else { Ok(k) })
                                    .map(Curve::Composite)
                                    .map_err(|e| e.rule().to_string()),
                            )
                        }
                    }
                };
                if let Some(result) = built {
                    let id = view.record(index).map_or(0, |r| r.id);
                    slots[index] = match result {
                        Ok(c) => {
                            out.curves.push((id, c.clone()));
                            Slot::Done(c)
                        }
                        Err(reason) => {
                            out.unsupported.push((id, reason));
                            Slot::Failed
                        }
                    };
                    settled = true;
                }
            }
        }
        #[allow(clippy::needless_range_loop)]
        for index in 0..view.len() {
            if matches!(slots[index], Slot::Pending) {
                out.unsupported
                    .push((view.record(index).map_or(0, |r| r.id), "an unresolved segment".to_string()));
            }
        }
    }
    out.curves.sort_by_key(|(id, _)| *id);
    out.unsupported.sort();
    Ok(out)
}
