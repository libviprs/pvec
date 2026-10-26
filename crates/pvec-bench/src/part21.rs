//! The Part 21 arm: the shapes of an artifact exported through the P6.2 writer, and a reference parse of the
//! text that a Part 21 reader cannot be faster than.
//!
//! The parse is not the independent evaluator. The evaluator is OCCT in the `tools/step-eval` image, and its
//! read time is what the cold-open threshold is judged on. This parse is the floor under any reader: it has
//! to find every instance and turn every parameter into a value, and it does nothing else, so a `.pvec` that
//! opens ten times faster than even this has not been given an easy opponent.

use crate::shape::Shape;
use pvec_entity::EntityId;
use pvec_geometry::{
    BSpline, Circle, Curve, Ellipse, Line, Placement2, Point2, Polyline, RationalBSpline, Trimmed, Vector2,
};
use pvec_interchange::step::{StepEntity, StepOptions, export};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

fn point(p: [f64; 2]) -> Point2 {
    Point2::new(p[0], p[1])
}

/// The exact curve P6.2 writes for a shape. A segment is a trimmed line, a full circle or ellipse is the
/// conic, a partial one is a trimmed conic, and a clockwise arc is a trim with the sense reversed.
fn curve(shape: &Shape) -> Result<Curve, String> {
    let g = |e: pvec_geometry::GeometryError| e.to_string();
    match shape {
        Shape::Line(a, b) => {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let line = Line::new(point(*a), Vector2::new(dx, dy)).map_err(g)?;
            Trimmed::new(Curve::Line(line), 0.0, (dx * dx + dy * dy).sqrt(), true)
                .map(Curve::Trimmed)
                .map_err(g)
        }
        Shape::Polyline(p) => {
            Polyline::new(p.iter().map(|q| point(*q)).collect()).map(Curve::Polyline).map_err(g)
        }
        Shape::Arc { centre, radius, start_sweep: [start, sweep] } => {
            let place = Placement2::new(point(*centre), Vector2::new(1.0, 0.0)).map_err(g)?;
            let circle = Curve::Circle(Circle::new(place, *radius).map_err(g)?);
            if sweep.abs() == TAU {
                Ok(circle)
            } else {
                Trimmed::new(circle, *start, start + sweep, *sweep > 0.0).map(Curve::Trimmed).map_err(g)
            }
        }
        Shape::Ellipse { centre, radii, angles: [rotation, start, sweep] } => {
            let axis = if *rotation == 0.0 {
                Vector2::new(1.0, 0.0)
            } else {
                Vector2::new(rotation.cos(), rotation.sin())
            };
            let place = Placement2::new(point(*centre), axis).map_err(g)?;
            let ellipse = Curve::Ellipse(Ellipse::new(place, radii[0], radii[1]).map_err(g)?);
            if sweep.abs() == TAU {
                Ok(ellipse)
            } else {
                Trimmed::new(ellipse, *start, start + sweep, *sweep > 0.0).map(Curve::Trimmed).map_err(g)
            }
        }
        Shape::Spline(s) => {
            let cps: Vec<Point2> = s.x.iter().zip(&s.y).map(|(x, y)| Point2::new(*x, *y)).collect();
            match &s.weights {
                Some(w) => RationalBSpline::new(s.degree, cps, s.knots.clone(), w.clone())
                    .map(Curve::RationalBSpline)
                    .map_err(g),
                None => BSpline::new(s.degree, cps, s.knots.clone()).map(Curve::BSpline).map_err(g),
            }
        }
    }
}

/// The Part 21 text of the shapes, or why P6.2 cannot hold them: the number of entities it refuses, by reason.
pub fn text(shapes: &[Shape]) -> Result<String, String> {
    let mut entities = Vec::with_capacity(shapes.len());
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for (i, s) in shapes.iter().enumerate() {
        match curve(s) {
            Ok(c) => entities.push(StepEntity::exact(EntityId::new(i as u32).map_err(|e| e.to_string())?, c)),
            Err(why) => *refused.entry(why).or_insert(0) += 1,
        }
    }
    if !refused.is_empty() {
        let list: Vec<String> = refused.iter().map(|(why, n)| format!("{n} x {why}")).collect();
        return Err(format!(
            "P6.2 cannot hold {} of {} entities: {}",
            refused.values().sum::<usize>(),
            shapes.len(),
            list.join("; ")
        ));
    }
    let options = StepOptions::new("corpus.stp", "2026-10-25T00:00:00").map_err(|e| e.to_string())?;
    export(&entities, &options).map(|o| o.part21).map_err(|e| e.to_string())
}

/// What the reference parse found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// Instances of the data section.
    pub instances: u64,
    /// Parameters turned into values, nested ones included.
    pub parameters: u64,
    /// Reals among them.
    pub reals: u64,
    /// A fold of every value read, so the parse cannot be optimised away and two parses can be compared.
    pub checksum: u64,
}

enum Param {
    Ref(u32),
    Real(f64),
    Int(i64),
    Text,
    Enum,
    List(Vec<Param>),
    Null,
}

fn count(p: &Param, scan: &mut Scan) {
    scan.parameters += 1;
    let mut mix = |bits: u64| scan.checksum = (scan.checksum ^ bits).wrapping_mul(0x0000_0100_0000_01b3);
    match p {
        Param::Real(r) => {
            mix(r.to_bits());
            scan.reals += 1;
        }
        Param::Int(i) => mix(*i as u64),
        Param::Ref(r) => mix(u64::from(*r)),
        Param::List(l) => l.iter().for_each(|q| count(q, scan)),
        Param::Text | Param::Enum | Param::Null => {}
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn ws(&mut self) {
        while self.at < self.b.len() && self.b[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }
    fn word(&mut self) -> &str {
        let from = self.at;
        while self.at < self.b.len() && (self.b[self.at].is_ascii_alphanumeric() || self.b[self.at] == b'_') {
            self.at += 1;
        }
        std::str::from_utf8(&self.b[from..self.at]).unwrap_or("")
    }
    fn param(&mut self) -> Result<Param, String> {
        self.ws();
        let c = *self.b.get(self.at).ok_or("text ends inside a parameter")?;
        match c {
            b'$' | b'*' => {
                self.at += 1;
                Ok(Param::Null)
            }
            b'\'' => {
                self.at += 1;
                while self.at < self.b.len() {
                    if self.b[self.at] == b'\'' {
                        if self.b.get(self.at + 1) == Some(&b'\'') {
                            self.at += 2;
                            continue;
                        }
                        self.at += 1;
                        return Ok(Param::Text);
                    }
                    self.at += 1;
                }
                Err("text ends inside a string".into())
            }
            b'.' => {
                self.at += 1;
                while self.at < self.b.len() && self.b[self.at] != b'.' {
                    self.at += 1;
                }
                self.at += 1;
                Ok(Param::Enum)
            }
            b'#' => {
                self.at += 1;
                let from = self.at;
                while self.at < self.b.len() && self.b[self.at].is_ascii_digit() {
                    self.at += 1;
                }
                std::str::from_utf8(&self.b[from..self.at])
                    .unwrap_or("")
                    .parse()
                    .map(Param::Ref)
                    .map_err(|e| format!("{e}"))
            }
            b'(' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    match self.b.get(self.at) {
                        Some(b')') => {
                            self.at += 1;
                            return Ok(Param::List(items));
                        }
                        Some(b',') => self.at += 1,
                        Some(_) => items.push(self.param()?),
                        None => return Err("text ends inside a list".into()),
                    }
                }
            }
            b'-' | b'+' | b'0'..=b'9' => {
                let from = self.at;
                while self.at < self.b.len()
                    && matches!(self.b[self.at], b'-' | b'+' | b'.' | b'E' | b'e' | b'0'..=b'9')
                {
                    self.at += 1;
                }
                let t = std::str::from_utf8(&self.b[from..self.at]).unwrap_or("");
                match t.parse::<i64>() {
                    Ok(i) => Ok(Param::Int(i)),
                    Err(_) => t
                        .trim_end_matches('.')
                        .parse::<f64>()
                        .map(Param::Real)
                        .map_err(|e| format!("{t}: {e}")),
                }
            }
            _ => {
                // a typed parameter such as LENGTH_MEASURE(1.0): the name, then its list
                let name = self.word().to_string();
                if name.is_empty() {
                    return Err(format!("unexpected byte {:?}", c as char));
                }
                self.ws();
                self.param()
            }
        }
    }
}

/// Parses every instance of the data section into values. The instance ids are checked to be unique, which
/// is what a reader needs to resolve a reference.
pub fn scan(text: &str) -> Result<Scan, String> {
    let data = text.find("DATA;").ok_or("no DATA section")? + "DATA;".len();
    let end = text.rfind("ENDSEC;").ok_or("no ENDSEC")?;
    let mut c = Cursor { b: text[data..end].as_bytes(), at: 0 };
    let mut scan = Scan::default();
    let mut seen = std::collections::HashSet::new();
    loop {
        c.ws();
        if c.at >= c.b.len() {
            return Ok(scan);
        }
        if c.b[c.at] != b'#' {
            return Err(format!("expected an instance at byte {}", c.at));
        }
        let Param::Ref(id) = c.param()? else { return Err("instance id".into()) };
        if !seen.insert(id) {
            return Err(format!("instance #{id} twice"));
        }
        c.ws();
        if c.b.get(c.at) != Some(&b'=') {
            return Err(format!("#{id}: expected `=`"));
        }
        c.at += 1;
        c.ws();
        // a simple instance is NAME(params); a complex one is ( NAME(params) NAME(params) )
        let complex = c.b.get(c.at) == Some(&b'(');
        if complex {
            c.at += 1;
        }
        loop {
            c.ws();
            if complex && c.b.get(c.at) == Some(&b')') {
                c.at += 1;
                break;
            }
            if c.word().is_empty() {
                return Err(format!("#{id}: no entity name"));
            }
            c.ws();
            let p = c.param()?;
            count(&p, &mut scan);
            if !complex {
                break;
            }
        }
        c.ws();
        if c.b.get(c.at) != Some(&b';') {
            return Err(format!("#{id}: expected `;`"));
        }
        c.at += 1;
        scan.instances += 1;
    }
}
