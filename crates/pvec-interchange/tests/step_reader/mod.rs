//! A small Part 21 reader for the STEP export tests (pvec#19).
//!
//! This is the in-process reader: it knows nothing of the writer's internals, parses only what the
//! file says, and rebuilds PVec curves through the public constructors, so a round trip through it
//! shows which parameters survive. It is not the independent STEP implementation of the acceptance
//! criteria; that one runs in a container (`step_19_reader.rs`).
#![allow(dead_code)]

use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve, Ellipse, Line, Placement2, Point2, Polyline, RationalBSpline,
    Trimmed, Vector2,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Ref(u32),
    Int(i64),
    Real(f64),
    Str(String),
    Enum(String),
    Unset,
    List(Vec<Val>),
    Typed(String, Vec<Val>),
}

#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub args: Vec<Val>,
}

#[derive(Clone, Debug)]
pub struct Instance {
    pub id: u32,
    pub parts: Vec<Part>,
}

#[derive(Debug)]
pub struct File {
    pub header: String,
    pub instances: Vec<Instance>,
}

struct Cur<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: u8) {
        assert!(self.eat(c), "expected {:?} at byte {}", c as char, self.i);
    }
    fn word(&mut self) -> String {
        self.ws();
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        String::from_utf8(self.s[st..self.i].to_vec()).unwrap()
    }
    fn value(&mut self) -> Val {
        match self.peek().expect("a value") {
            b'#' => {
                self.i += 1;
                Val::Ref(self.word().parse().unwrap())
            }
            b'\'' => {
                self.i += 1;
                let mut o = Vec::new();
                loop {
                    let c = self.s[self.i];
                    self.i += 1;
                    if c == b'\'' {
                        if self.s.get(self.i) == Some(&b'\'') {
                            o.push(b'\'');
                            self.i += 1;
                        } else {
                            break;
                        }
                    } else {
                        o.push(c);
                    }
                }
                Val::Str(String::from_utf8(o).unwrap())
            }
            b'.' => {
                self.i += 1;
                let st = self.i;
                while self.s[self.i] != b'.' {
                    self.i += 1;
                }
                let e = String::from_utf8(self.s[st..self.i].to_vec()).unwrap();
                self.i += 1;
                Val::Enum(e)
            }
            b'$' | b'*' => {
                self.i += 1;
                Val::Unset
            }
            b'(' => {
                self.i += 1;
                let mut v = Vec::new();
                if !self.eat(b')') {
                    loop {
                        v.push(self.value());
                        if self.eat(b')') {
                            break;
                        }
                        self.expect(b',');
                    }
                }
                Val::List(v)
            }
            b'+' | b'-' | b'0'..=b'9' => {
                let st = self.i;
                while self.i < self.s.len()
                    && matches!(self.s[self.i], b'+' | b'-' | b'0'..=b'9' | b'.' | b'E')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[st..self.i]).unwrap();
                if t.contains('.') || t.contains('E') {
                    Val::Real(t.parse().unwrap_or_else(|_| panic!("bad real {t}")))
                } else {
                    Val::Int(t.parse().unwrap())
                }
            }
            _ => {
                let name = self.word();
                self.expect(b'(');
                let mut v = Vec::new();
                if !self.eat(b')') {
                    loop {
                        v.push(self.value());
                        if self.eat(b')') {
                            break;
                        }
                        self.expect(b',');
                    }
                }
                Val::Typed(name, v)
            }
        }
    }
    fn part(&mut self) -> Part {
        let name = self.word();
        self.expect(b'(');
        let mut args = Vec::new();
        if !self.eat(b')') {
            loop {
                args.push(self.value());
                if self.eat(b')') {
                    break;
                }
                self.expect(b',');
            }
        }
        Part { name, args }
    }
}

pub fn parse(text: &str) -> File {
    assert!(text.starts_with("ISO-10303-21;\n"), "not a Part 21 file");
    assert!(text.trim_end().ends_with("END-ISO-10303-21;"), "no trailer");
    let (head, rest) = text.split_once("\nDATA;\n").expect("a DATA section");
    let (data, _) = rest.split_once("\nENDSEC;\n").expect("DATA ends");
    let mut c = Cur { s: data.as_bytes(), i: 0 };
    let mut instances = Vec::new();
    while c.peek().is_some() {
        c.expect(b'#');
        let id: u32 = c.word().parse().unwrap();
        c.expect(b'=');
        let parts = if c.eat(b'(') {
            let mut ps = Vec::new();
            while !c.eat(b')') {
                ps.push(c.part());
            }
            ps
        } else {
            vec![c.part()]
        };
        c.expect(b';');
        instances.push(Instance { id, parts });
    }
    File { header: head.to_string(), instances }
}

impl File {
    pub fn get(&self, id: u32) -> &Instance {
        self.instances.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no instance #{id}"))
    }

    /// The instance whose first part is `name` and whose name attribute is `label`.
    pub fn by_label(&self, label: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| {
            i.parts.iter().any(|p| {
                matches!(p.args.first(), Some(Val::Str(s)) if s == label)
                    && p.name != "REPRESENTATION_CONTEXT"
            })
        })
    }
}

fn num(v: &Val) -> f64 {
    match v {
        Val::Real(r) => *r,
        Val::Int(i) => *i as f64,
        Val::Typed(n, a) if n == "PARAMETER_VALUE" => num(&a[0]),
        other => panic!("not a number: {other:?}"),
    }
}

fn r(v: &Val) -> u32 {
    match v {
        Val::Ref(n) => *n,
        other => panic!("not a reference: {other:?}"),
    }
}

fn list(v: &Val) -> &[Val] {
    match v {
        Val::List(l) => l,
        other => panic!("not a list: {other:?}"),
    }
}

impl File {
    fn point(&self, id: u32) -> Point2 {
        let p = &self.get(id).parts[0];
        assert_eq!(p.name, "CARTESIAN_POINT");
        let c = list(&p.args[1]);
        assert_eq!(c.len(), 2, "a 2D point has two coordinates");
        Point2::new(num(&c[0]), num(&c[1]))
    }

    fn direction(&self, id: u32) -> Vector2 {
        let p = &self.get(id).parts[0];
        assert_eq!(p.name, "DIRECTION");
        let c = list(&p.args[1]);
        Vector2::new(num(&c[0]), num(&c[1]))
    }

    fn placement(&self, id: u32) -> Placement2 {
        let p = &self.get(id).parts[0];
        assert_eq!(p.name, "AXIS2_PLACEMENT_2D");
        Placement2::from_stored(self.point(r(&p.args[1])), self.direction(r(&p.args[2]))).unwrap()
    }

    fn knots(mults: &Val, values: &Val) -> Vec<f64> {
        let mut k = Vec::new();
        for (m, v) in list(mults).iter().zip(list(values)) {
            let Val::Int(m) = m else { panic!("multiplicity is an integer") };
            for _ in 0..*m {
                k.push(num(v));
            }
        }
        k
    }

    /// The PVec curve a curve instance describes.
    pub fn curve(&self, id: u32) -> Curve {
        let inst = self.get(id);
        if inst.parts.len() > 1 {
            let find = |n: &str| inst.parts.iter().find(|p| p.name == n).unwrap_or_else(|| panic!("no {n}"));
            let bs = find("B_SPLINE_CURVE");
            let wk = find("B_SPLINE_CURVE_WITH_KNOTS");
            let rat = find("RATIONAL_B_SPLINE_CURVE");
            let degree = match bs.args[0] {
                Val::Int(d) => d as usize,
                _ => panic!("degree"),
            };
            let cps = list(&bs.args[1]).iter().map(|v| self.point(r(v))).collect();
            let knots = File::knots(&wk.args[0], &wk.args[1]);
            let w = list(&rat.args[0]).iter().map(num).collect();
            let curve = RationalBSpline::new(degree, cps, knots, w).unwrap();
            let closed = matches!(&bs.args[3], Val::Enum(e) if e == "T");
            return Curve::RationalBSpline(if closed { curve.declare_closed().unwrap() } else { curve });
        }
        let p = &inst.parts[0];
        match p.name.as_str() {
            "LINE" => {
                let v = &self.get(r(&p.args[2])).parts[0];
                assert_eq!(v.name, "VECTOR");
                assert_eq!(num(&v.args[2]), 1.0, "a line's vector has magnitude 1");
                Curve::Line(
                    Line::from_stored(self.point(r(&p.args[1])), self.direction(r(&v.args[1]))).unwrap(),
                )
            }
            "CIRCLE" => Curve::Circle(Circle::new(self.placement(r(&p.args[1])), num(&p.args[2])).unwrap()),
            "ELLIPSE" => Curve::Ellipse(
                Ellipse::new(self.placement(r(&p.args[1])), num(&p.args[2]), num(&p.args[3])).unwrap(),
            ),
            "POLYLINE" => Curve::Polyline(
                Polyline::new(list(&p.args[1]).iter().map(|v| self.point(r(v))).collect()).unwrap(),
            ),
            "B_SPLINE_CURVE_WITH_KNOTS" => {
                let Val::Int(d) = p.args[1] else { panic!("degree") };
                let cps: Vec<Point2> = list(&p.args[2]).iter().map(|v| self.point(r(v))).collect();
                let knots = File::knots(&p.args[6], &p.args[7]);
                let curve = BSpline::new(d as usize, cps, knots).unwrap();
                let closed = matches!(&p.args[4], Val::Enum(e) if e == "T");
                Curve::BSpline(if closed { curve.declare_closed().unwrap() } else { curve })
            }
            "TRIMMED_CURVE" => {
                let basis = self.curve(r(&p.args[1]));
                let t1 = num(&list(&p.args[2])[0]);
                let t2 = num(&list(&p.args[3])[0]);
                let sense = matches!(&p.args[4], Val::Enum(e) if e == "T");
                assert!(matches!(&p.args[5], Val::Enum(e) if e == "PARAMETER"), "master representation");
                Curve::Trimmed(Trimmed::new(basis, t1, t2, sense).unwrap())
            }
            "COMPOSITE_CURVE" => {
                let mut segs = Vec::new();
                for s in list(&p.args[1]) {
                    let seg = &self.get(r(s)).parts[0];
                    assert_eq!(seg.name, "COMPOSITE_CURVE_SEGMENT");
                    segs.push(self.curve(r(&seg.args[2])));
                }
                Curve::Composite(Composite::new(segs).unwrap())
            }
            other => panic!("no reader for {other}"),
        }
    }

    /// Every curve whose name attribute is `e<n>`, as `(n, instance id)`, in file order.
    pub fn entities(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for i in &self.instances {
            let name = i.parts.iter().find_map(|p| match p.args.first() {
                Some(Val::Str(s)) => Some(s.clone()),
                _ => None,
            });
            if let Some(n) = name.as_deref().and_then(|s| s.strip_prefix('e')) {
                if let Ok(n) = n.parse::<u32>() {
                    out.push((n, i.id));
                }
            }
        }
        out
    }
}

/// A Bezier as the clamped single-span B-spline the file carries.
pub fn as_bspline(b: &Bezier) -> Curve {
    let d = b.degree();
    let mut k = vec![0.0; d + 1];
    k.extend(vec![1.0; d + 1]);
    Curve::BSpline(BSpline::new(d, b.control_points().to_vec(), k).unwrap())
}
