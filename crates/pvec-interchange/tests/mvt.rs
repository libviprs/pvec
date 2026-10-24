//! The display-only MVT export (issue P6.6, pvec#23).
//!
//! The decoder at the top is written from the vector tile specification and shares no code with the
//! encoder in `src/mvt.rs`: it is the independent reader the acceptance criteria ask for.

use pvec_entity::EntityId;
use pvec_geometry::{
    Bezier, Circle, Composite, Curve, Curve2d, Line, Placement2, Point2, Polyline, Transform2, Trimmed,
    Vector2,
};
use pvec_interchange::mvt::*;
use pvec_interchange::{Fidelity, FidelityKind, FidelityRecord, Representation, TargetConstruct};
use pvec_numeric::Policy;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

// ---- an independent decoder -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum DVal {
    Uint(u64),
    Double(f64),
    Bool(bool),
}

#[derive(Debug)]
struct DFeature {
    id: u64,
    tags: BTreeMap<String, DVal>,
    geom_type: u64,
    lines: Vec<Vec<(i64, i64)>>,
}

#[derive(Debug)]
struct DLayer {
    name: String,
    version: u64,
    extent: u64,
    features: Vec<DFeature>,
}

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Rd<'a> {
    fn varint(&mut self) -> u64 {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = *self.b.get(self.i).expect("truncated varint");
            self.i += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return v;
            }
            shift += 7;
            assert!(shift < 70, "varint too long");
        }
    }
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
    fn field(&mut self) -> (u64, u64) {
        let k = self.varint();
        (k >> 3, k & 7)
    }
    fn bytes(&mut self) -> &'a [u8] {
        let n = self.varint() as usize;
        let s = self.b.get(self.i..self.i + n).expect("truncated length-delimited field");
        self.i += n;
        s
    }
    fn fixed64(&mut self) -> [u8; 8] {
        let s: [u8; 8] = self.b.get(self.i..self.i + 8).expect("truncated fixed64").try_into().unwrap();
        self.i += 8;
        s
    }
}

fn unzig(v: u32) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn decode_value(b: &[u8]) -> DVal {
    let mut r = Rd { b, i: 0 };
    let (f, w) = r.field();
    let v = match (f, w) {
        (3, 1) => DVal::Double(f64::from_le_bytes(r.fixed64())),
        (5, 0) => DVal::Uint(r.varint()),
        (7, 0) => DVal::Bool(r.varint() != 0),
        other => panic!("unexpected value field {other:?}"),
    };
    assert!(r.done(), "trailing bytes in a value");
    v
}

fn decode_geometry(cmds: &[u32], extent: u64, buffer: i64) -> Vec<Vec<(i64, i64)>> {
    let mut lines: Vec<Vec<(i64, i64)>> = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let mut i = 0;
    while i < cmds.len() {
        let id = cmds[i] & 7;
        let count = (cmds[i] >> 3) as usize;
        i += 1;
        match id {
            1 => {
                assert_eq!(count, 1, "a linestring MoveTo has count 1");
                assert!(i + 2 <= cmds.len(), "MoveTo parameters missing");
                x += unzig(cmds[i]);
                y += unzig(cmds[i + 1]);
                i += 2;
                lines.push(vec![(x, y)]);
            }
            2 => {
                assert!(count >= 1, "LineTo with no points");
                assert!(i + 2 * count <= cmds.len(), "LineTo parameters missing");
                let line = lines.last_mut().expect("LineTo before MoveTo");
                for _ in 0..count {
                    x += unzig(cmds[i]);
                    y += unzig(cmds[i + 1]);
                    i += 2;
                    line.push((x, y));
                }
            }
            other => panic!("command {other} is not allowed in a linestring"),
        }
    }
    for l in &lines {
        assert!(l.len() >= 2, "a linestring needs two points");
        for &(px, py) in l {
            let lo = -buffer;
            let hi = extent as i64 + buffer;
            assert!(
                (lo..=hi).contains(&px) && (lo..=hi).contains(&py),
                "vertex ({px},{py}) outside the buffered tile"
            );
        }
    }
    lines
}

fn decode_tile(bytes: &[u8], buffer: i64) -> Vec<DLayer> {
    let mut r = Rd { b: bytes, i: 0 };
    let mut layers = Vec::new();
    while !r.done() {
        let (f, w) = r.field();
        assert_eq!((f, w), (3, 2), "a tile holds only layers");
        layers.push(decode_layer(r.bytes(), buffer));
    }
    layers
}

fn decode_layer(b: &[u8], buffer: i64) -> DLayer {
    let mut r = Rd { b, i: 0 };
    let (mut name, mut version, mut extent) = (None, None, None);
    let (mut keys, mut values, mut raw_feats) = (Vec::new(), Vec::new(), Vec::new());
    while !r.done() {
        let (f, w) = r.field();
        match (f, w) {
            (1, 2) => name = Some(String::from_utf8(r.bytes().to_vec()).expect("utf8 name")),
            (2, 2) => raw_feats.push(r.bytes()),
            (3, 2) => keys.push(String::from_utf8(r.bytes().to_vec()).expect("utf8 key")),
            (4, 2) => values.push(decode_value(r.bytes())),
            (5, 0) => extent = Some(r.varint()),
            (15, 0) => version = Some(r.varint()),
            other => panic!("unexpected layer field {other:?}"),
        }
    }
    let name = name.expect("layer name");
    assert!(!name.is_empty());
    let version = version.expect("layer version");
    assert_eq!(version, 2);
    let extent = extent.expect("layer extent");
    let mut features = Vec::new();
    for fb in raw_feats {
        let mut r = Rd { b: fb, i: 0 };
        let (mut id, mut tags, mut gt, mut geom) = (0u64, Vec::new(), 0u64, Vec::new());
        while !r.done() {
            let (f, w) = r.field();
            match (f, w) {
                (1, 0) => id = r.varint(),
                (2, 2) => {
                    let mut p = Rd { b: r.bytes(), i: 0 };
                    while !p.done() {
                        tags.push(p.varint() as usize);
                    }
                }
                (3, 0) => gt = r.varint(),
                (4, 2) => {
                    let mut p = Rd { b: r.bytes(), i: 0 };
                    while !p.done() {
                        geom.push(p.varint() as u32);
                    }
                }
                other => panic!("unexpected feature field {other:?}"),
            }
        }
        assert_eq!(tags.len() % 2, 0, "tags come in pairs");
        let mut map = BTreeMap::new();
        for pair in tags.chunks(2) {
            assert!(pair[0] < keys.len() && pair[1] < values.len(), "tag index out of range");
            map.insert(keys[pair[0]].clone(), values[pair[1]].clone());
        }
        features.push(DFeature {
            id,
            tags: map,
            geom_type: gt,
            lines: decode_geometry(&geom, extent, buffer),
        });
    }
    DLayer { name, version, extent, features }
}

// ---- the named fixture -----------------------------------------------------------------------------

fn eid(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn circle(cx: f64, cy: f64, r: f64) -> Curve {
    Curve::Circle(Circle::new(Placement2::new(p(cx, cy), Vector2::new(1.0, 0.0)).unwrap(), r).unwrap())
}

/// The named fixture `crossing-curves`: a drawing 200 units across, placed on a 1024 unit map with a
/// scale of 2, a rotation of 0.4 rad and a shift, so that at zoom 2 its curves cross many tile edges.
fn fixture_projection() -> Projection {
    let t = Transform2::scale(2.0, 2.0)
        .unwrap()
        .then(&Transform2::rotate(0.4).unwrap())
        .unwrap()
        .then(&Transform2::translate(512.0, 100.0).unwrap())
        .unwrap();
    Projection::new(t, 1024.0, 512.0).unwrap()
}

fn fixture() -> Vec<MvtEntity> {
    let arc = Curve::Trimmed(Trimmed::from_bulge(p(20.0, 150.0), p(180.0, 160.0), 0.8).unwrap());
    let bez = Curve::Bezier(
        Bezier::new(vec![p(0.0, 0.0), p(60.0, 190.0), p(140.0, -30.0), p(200.0, 120.0)]).unwrap(),
    );
    let poly = Curve::Polyline(
        Polyline::new(vec![p(0.0, 100.0), p(70.0, 30.0), p(130.0, 170.0), p(200.0, 90.0)]).unwrap(),
    );
    let comp = Curve::Composite(
        Composite::new(vec![
            Curve::Polyline(Polyline::new(vec![p(10.0, 10.0), p(60.0, 20.0)]).unwrap()),
            Curve::Trimmed(Trimmed::from_bulge(p(60.0, 20.0), p(110.0, 70.0), -0.5).unwrap()),
        ])
        .unwrap(),
    );
    vec![
        MvtEntity::new(eid(1), "circles", circle(100.0, 100.0, 80.0)).unwrap(),
        MvtEntity::new(eid(2), "circles", arc).unwrap(),
        MvtEntity::new(eid(3), "splines", bez).unwrap(),
        MvtEntity::new(eid(4), "lines", poly).unwrap(),
        MvtEntity::new(eid(5), "lines", comp).unwrap(),
    ]
}

fn curves() -> Vec<Curve> {
    fixture().iter().map(e_curve).collect()
}

fn e_curve(e: &MvtEntity) -> Curve {
    e.curve().clone()
}

fn dist_to_polyline(q: Point2, pts: &[Point2]) -> f64 {
    pts.windows(2)
        .map(|w| {
            let (dx, dy) = (w[1].x - w[0].x, w[1].y - w[0].y);
            let l2 = dx * dx + dy * dy;
            let t = if l2 == 0.0 {
                0.0
            } else {
                (((q.x - w[0].x) * dx + (q.y - w[0].y) * dy) / l2).clamp(0.0, 1.0)
            };
            q.distance(p(w[0].x + t * dx, w[0].y + t * dy))
        })
        .fold(f64::INFINITY, f64::min)
}

fn dense(c: &Curve, n: usize) -> Vec<Point2> {
    let d = c.domain();
    (0..=n)
        .map(|k| {
            let t = if k == n { d.hi() } else { d.lo() + (d.hi() - d.lo()) * k as f64 / n as f64 };
            c.eval(t).unwrap()
        })
        .collect()
}

// ---- no MVT entity can be Exact --------------------------------------------------------------------

#[test]
fn issue_23_exact_claims_fail_at_construction() {
    let mut ledger = MvtLedger::new();
    for f in [Fidelity::Exact, Fidelity::ExactReparameterized, Fidelity::EquivalentCompound] {
        let kind = f.kind();
        let err = ledger.record(eid(1), pvec_capabilities::ids::CIRCLE, f, "x").unwrap_err();
        assert_eq!(err, MvtError::NotDisplayOnly(kind));
    }
    assert!(ledger.ledger().records().is_empty());
    // and the general rule of the taxonomy still holds underneath
    let t = TargetConstruct::new("mvt.LineString", Representation::Tessellated).unwrap();
    assert!(FidelityRecord::new(eid(1), pvec_capabilities::ids::CIRCLE, t, Fidelity::Exact, "x").is_err());
}

#[test]
fn issue_23_every_exported_record_is_bounded_or_unsupported() {
    let mut ents = fixture();
    ents.push(
        MvtEntity::new(eid(9), "lines", Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap()))
            .unwrap(),
    );
    let pr = fixture_projection();
    let mut seen = 0;
    for z in 0..=4u8 {
        for x in 0..(1u32 << z) {
            for y in 0..(1u32 << z) {
                let t =
                    export_tile(&ents, &pr, TileId::new(z, x, y).unwrap(), &MvtOptions::default()).unwrap();
                for r in t.ledger.ledger().records() {
                    seen += 1;
                    assert!(matches!(
                        r.fidelity().kind(),
                        FidelityKind::BoundedApproximation | FidelityKind::Unsupported
                    ));
                    assert_eq!(r.target_construct().representation(), Representation::Tessellated);
                }
            }
        }
    }
    assert!(seen > 20);
}

#[test]
fn issue_23_unbounded_line_is_unsupported_with_a_reason() {
    let line = Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap());
    let ents = [MvtEntity::new(eid(7), "l", line).unwrap()];
    let t = export_tile(&ents, &fixture_projection(), TileId::new(0, 0, 0).unwrap(), &MvtOptions::default())
        .unwrap();
    let recs = t.ledger.ledger().records();
    assert_eq!(recs.len(), 1);
    match recs[0].fidelity() {
        Fidelity::Unsupported(r) => assert!(r.as_str().contains("unbounded")),
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert!(t.bytes.is_empty());
}

// ---- deviation against exact evaluation ------------------------------------------------------------

#[test]
fn issue_23_tessellation_stays_within_the_screen_derived_tolerance_across_zooms() {
    let pr = fixture_projection();
    let opts = MvtOptions::default();
    for z in 0..=MAX_TESTED_ZOOM {
        let tol = pr.tessellation_tolerance(z, &opts).unwrap();
        let budget = pr.screen_to_world(z, opts.screen_tolerance_px);
        assert!(tol < budget);
        for c in curves() {
            let Ok(t) = tessellate(&c, tol) else {
                panic!("zoom {z}: tessellation failed");
            };
            assert!(t.deviation <= tol, "zoom {z}: measured {} > tolerance {tol}", t.deviation);
            // an independent check against dense exact evaluation, not the samples the tessellator used
            let worst = dense(&c, 4000).iter().map(|q| dist_to_polyline(*q, &t.points)).fold(0.0, f64::max);
            assert!(
                worst <= tol * (1.0 + Policy::V1.rel * 1000.0),
                "zoom {z}: dense deviation {worst} > tolerance {tol}"
            );
            // the polyline starts and ends where the curve does
            let d = c.domain();
            assert!(t.points[0].distance(c.eval(d.lo()).unwrap()) < Policy::V1.abs * 1000.0);
            assert!(t.points.last().unwrap().distance(c.eval(d.hi()).unwrap()) < Policy::V1.abs * 1000.0);
        }
    }
}

#[test]
fn issue_23_ledger_bound_covers_what_the_tile_actually_holds() {
    let pr = fixture_projection();
    let opts = MvtOptions::default();
    let ents = fixture();
    let z = 3u8;
    let mut checked = 0;
    for x in 0..(1u32 << z) {
        for y in 0..(1u32 << z) {
            let tile = TileId::new(z, x, y).unwrap();
            let t = export_tile(&ents, &pr, tile, &opts).unwrap();
            let layers = decode_tile(&t.bytes, opts.buffer as i64);
            for l in &layers {
                for f in &l.features {
                    let rec = t
                        .ledger
                        .ledger()
                        .records()
                        .iter()
                        .find(|r| r.entity_id().index() as u64 == f.id)
                        .unwrap();
                    let bound = rec.error_bound().unwrap().get();
                    assert!(
                        bound <= t.world_budget * (1.0 + Policy::V1.rel * 1000.0),
                        "bound {bound} above the screen budget {}",
                        t.world_budget
                    );
                    let curve = ents.iter().find(|e| e.id().index() as u64 == f.id).unwrap().curve().clone();
                    let exact = dense(&curve, 20000);
                    // every decoded vertex, mapped back to the drawing, lies within the recorded bound of the exact curve
                    for line in &f.lines {
                        for &(vx, vy) in line {
                            let w = pr.from_tile(tile, opts.extent, (vx as f64, vy as f64)).unwrap();
                            let d = exact.iter().map(|q| q.distance(w)).fold(f64::INFINITY, f64::min);
                            // dense sampling step slack: the curve is at most ~1000 units long over 20000 steps
                            assert!(d <= bound + 0.06, "vertex {d} from the curve, bound {bound}");
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 200);
}

// ---- quantization ------------------------------------------------------------------------------------

#[test]
fn issue_23_quantization_error_at_the_maximum_zoom_is_measured() {
    let pr = fixture_projection();
    let z = MAX_TESTED_ZOOM;
    let extent = DEFAULT_EXTENT;
    let bound = pr.quantization_error(z, extent);
    let c = circle(100.0, 100.0, 80.0);
    let mut worst = 0.0f64;
    let mut state = 0x9e3779b97f4a7c15u64;
    for _ in 0..5000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let t = (state >> 11) as f64 / (1u64 << 53) as f64 * std::f64::consts::TAU;
        let w = c.eval(t).unwrap();
        // the tile that contains the point
        let m = map_of(&pr, w);
        let size = 1024.0 / (1u64 << z) as f64;
        let tile = TileId::new(z, (m.0 / size) as u32, (m.1 / size) as u32).unwrap();
        let tu = pr.to_tile(tile, extent, w);
        let q = (tu.0.round(), tu.1.round());
        let back = pr.from_tile(tile, extent, q).unwrap();
        worst = worst.max(back.distance(w));
    }
    assert!(worst <= bound, "measured {worst:e} above the stated bound {bound:e}");
    assert!(
        worst > bound * 0.5,
        "the measurement should come close to the bound, got {worst:e} of {bound:e}"
    );
    println!("quantization at zoom {z}, extent {extent}: bound {bound:e} world units, measured {worst:e}");
}

fn map_of(pr: &Projection, w: Point2) -> (f64, f64) {
    pr.to_tile(TileId::new(0, 0, 0).unwrap(), 1024, w)
}

// ---- valid MVT, boundaries ------------------------------------------------------------------------

#[test]
fn issue_23_tiles_decode_and_features_carry_their_entity_id() {
    let pr = fixture_projection();
    let opts = MvtOptions::default();
    let ents = fixture();
    let mut features = 0;
    let mut names = std::collections::BTreeSet::new();
    for x in 0..4u32 {
        for y in 0..4u32 {
            let t = export_tile(&ents, &pr, TileId::new(2, x, y).unwrap(), &opts).unwrap();
            let layers = decode_tile(&t.bytes, opts.buffer as i64);
            let mut last = String::new();
            for l in &layers {
                assert!(l.name > last, "layers are in name order");
                last = l.name.clone();
                assert_eq!((l.version, l.extent), (2, 4096));
                names.insert(l.name.clone());
                for f in &l.features {
                    features += 1;
                    assert_eq!(f.geom_type, 2);
                    assert_eq!(f.tags["entity_id"], DVal::Uint(f.id));
                    assert_eq!(f.tags["display_only"], DVal::Bool(true));
                    let rec = t
                        .ledger
                        .ledger()
                        .records()
                        .iter()
                        .find(|r| r.entity_id().index() as u64 == f.id)
                        .unwrap();
                    match &f.tags["world_deviation"] {
                        DVal::Double(d) => {
                            assert_eq!(d.to_bits(), rec.error_bound().unwrap().get().to_bits())
                        }
                        other => panic!("{other:?}"),
                    }
                }
            }
        }
    }
    assert!(features > 10);
    assert_eq!(names.into_iter().collect::<Vec<_>>(), ["circles", "lines", "splines"]);
}

#[test]
fn issue_23_clipped_geometry_ends_on_the_tile_boundary() {
    let pr = fixture_projection();
    let opts = MvtOptions { buffer: 0, ..MvtOptions::default() };
    let ents = fixture();
    let mut on_edge = 0;
    for x in 0..4u32 {
        for y in 0..4u32 {
            let t = export_tile(&ents, &pr, TileId::new(2, x, y).unwrap(), &opts).unwrap();
            for l in decode_tile(&t.bytes, 0) {
                for f in l.features {
                    for line in f.lines {
                        // with no buffer every vertex is inside 0..=extent (decode_tile asserts it); count the ones on an edge
                        on_edge += line
                            .iter()
                            .filter(|v| v.0 == 0 || v.0 == 4096 || v.1 == 0 || v.1 == 4096)
                            .count();
                    }
                }
            }
        }
    }
    assert!(on_edge >= 8, "curves cross tile edges, so clipped pieces must end on them (saw {on_edge})");
}

// ---- no seam ------------------------------------------------------------------------------------------

#[test]
fn issue_23_adjacent_tiles_reconstruct_the_source_with_no_seam_or_gap() {
    let pr = fixture_projection();
    for buffer in [0u32, DEFAULT_BUFFER] {
        let opts = MvtOptions { buffer, ..MvtOptions::default() };
        let ents = fixture();
        let z = 2u8;
        // per entity, the world-space segments recovered from every tile
        let mut recovered: BTreeMap<u64, Vec<(Point2, Point2)>> = BTreeMap::new();
        let mut bounds: BTreeMap<u64, f64> = BTreeMap::new();
        for x in 0..(1u32 << z) {
            for y in 0..(1u32 << z) {
                let tile = TileId::new(z, x, y).unwrap();
                let t = export_tile(&ents, &pr, tile, &opts).unwrap();
                for r in t.ledger.ledger().records() {
                    let b = bounds.entry(r.entity_id().index() as u64).or_insert(0.0);
                    *b = b.max(r.error_bound().unwrap().get());
                }
                for l in decode_tile(&t.bytes, buffer as i64) {
                    for f in l.features {
                        for line in f.lines {
                            for w in line.windows(2) {
                                let a =
                                    pr.from_tile(tile, opts.extent, (w[0].0 as f64, w[0].1 as f64)).unwrap();
                                let b =
                                    pr.from_tile(tile, opts.extent, (w[1].0 as f64, w[1].1 as f64)).unwrap();
                                recovered.entry(f.id).or_default().push((a, b));
                            }
                        }
                    }
                }
            }
        }
        for e in &ents {
            let id = e.id().index() as u64;
            let segs = &recovered[&id];
            let bound = bounds[&id];
            // no gap: every point of the exact curve is within the bound of some recovered segment
            for q in dense(e.curve(), 3000) {
                let d =
                    segs.iter().map(|(a, b)| dist_to_polyline(q, &[*a, *b])).fold(f64::INFINITY, f64::min);
                assert!(
                    d <= bound * (1.0 + Policy::V1.rel * 1000.0),
                    "entity {id}, buffer {buffer}: gap of {d} against a bound of {bound}"
                );
            }
        }
    }
}

// ---- determinism --------------------------------------------------------------------------------------

#[test]
fn issue_23_export_is_byte_identical_for_identical_input() {
    let pr = fixture_projection();
    let opts = MvtOptions::default();
    for (z, x, y) in [(0u8, 0u32, 0u32), (2, 1, 1), (2, 2, 1), (5, 17, 9), (14, 8000, 5000)] {
        let tile = TileId::new(z, x, y).unwrap();
        let a = export_tile(&fixture(), &pr, tile, &opts).unwrap();
        let b = export_tile(&fixture(), &pr, tile, &opts).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.ledger, b.ledger);
    }
}

// ---- the stages, one at a time ----------------------------------------------------------------------

#[test]
fn issue_23_geometry_commands_match_the_specification_example() {
    // the LINESTRING (2,2) (2,10) (10,10) example of the vector tile specification
    let cmds = encode_geometry(&[vec![(2, 2), (2, 10), (10, 10)]]);
    assert_eq!(cmds, [9, 4, 4, 18, 0, 16, 16, 0]);
    // two linestrings: the cursor carries over, so the second MoveTo is relative to the last LineTo
    let cmds = encode_geometry(&[vec![(2, 2), (2, 10)], vec![(1, 1), (3, 5)]]);
    assert_eq!(cmds, [9, 4, 4, 10, 0, 16, 9, 1, 17, 10, 4, 8]);
}

#[test]
fn issue_23_clip_splits_at_the_rectangle_and_keeps_continuous_runs() {
    let r = ClipRect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
    // fully inside: one piece, unchanged
    let inside = [(1.0, 1.0), (5.0, 5.0), (9.0, 2.0)];
    assert_eq!(clip_polyline(&inside, r), vec![inside.to_vec()]);
    // fully outside: nothing
    assert!(clip_polyline(&[(-5.0, -5.0), (-1.0, 20.0)], r).is_empty());
    // leaves and re-enters: two pieces, each ending exactly on the edge
    let out = clip_polyline(&[(5.0, 5.0), (15.0, 5.0), (15.0, 8.0), (5.0, 8.0)], r);
    assert_eq!(out, vec![vec![(5.0, 5.0), (10.0, 5.0)], vec![(10.0, 8.0), (5.0, 8.0)]]);
    // a segment that crosses straight through
    let out = clip_polyline(&[(-5.0, 5.0), (15.0, 5.0)], r);
    assert_eq!(out, vec![vec![(0.0, 5.0), (10.0, 5.0)]]);
    // touching a corner only yields a zero-length piece, which is dropped
    assert!(clip_polyline(&[(-5.0, 5.0), (5.0, -5.0)], r).is_empty());
}

#[test]
fn issue_23_quantize_rounds_and_drops_collapsed_runs() {
    let q = quantize(&[vec![(0.4, 0.4), (0.45, 0.2), (10.6, 3.2)], vec![(5.1, 5.1), (4.9, 4.9)]]);
    assert_eq!(q, vec![vec![(0, 0), (11, 3)]]);
}

#[test]
fn issue_23_tessellation_stage_in_isolation() {
    let c = circle(0.0, 0.0, 100.0);
    let coarse = tessellate(&c, 1.0).unwrap();
    let fine = tessellate(&c, 0.01).unwrap();
    assert!(fine.points.len() > coarse.points.len() * 3);
    assert!(coarse.deviation <= 1.0 && fine.deviation <= 0.01);
    assert!(
        (coarse.points[0].distance(*coarse.points.last().unwrap())) < Policy::V1.abs * 1000.0,
        "a circle closes"
    );
    assert_eq!(tessellate(&c, 0.0).unwrap_err(), TessError::BadTolerance);
    assert_eq!(tessellate(&c, f64::NAN).unwrap_err(), TessError::BadTolerance);
}

#[test]
fn issue_23_screen_budget_converts_through_the_projection() {
    let pr = fixture_projection();
    let opts = MvtOptions::default();
    // scale 2 (drawing to map), 1024 map units, 512 px tiles: one pixel at zoom 0 is 2/512*512... check directly
    assert_eq!(pr.scale(), 2.0);
    assert_eq!(pr.map_units_per_pixel(0), 1024.0 / 512.0);
    assert_eq!(pr.screen_to_world(0, 0.25), 0.25 * 2.0 / 2.0);
    assert_eq!(pr.screen_to_world(3, 0.25), 0.25 * 0.25 / 2.0);
    let tol = pr.tessellation_tolerance(3, &opts).unwrap();
    assert!((pr.screen_to_world(3, 0.25) - tol - pr.quantization_error(3, 4096)).abs() < Policy::V1.abs);
}

#[test]
fn issue_23_setup_errors() {
    assert_eq!(TileId::new(2, 4, 0).unwrap_err(), MvtError::BadTile);
    assert_eq!(TileId::new(25, 0, 0).unwrap_err(), MvtError::BadTile);
    let shear = Transform2::shear(0.5, 0.0).unwrap();
    assert_eq!(Projection::new(shear, 1024.0, 512.0).unwrap_err(), MvtError::NotSimilarity);
    let aniso = Transform2::scale(1.0, 2.0).unwrap();
    assert_eq!(Projection::new(aniso, 1024.0, 512.0).unwrap_err(), MvtError::NotSimilarity);
    assert!(Projection::new(Transform2::mirror_x().unwrap(), 1024.0, 512.0).is_ok());
    let coarse = MvtOptions { extent: 16, buffer: 0, ..MvtOptions::default() };
    assert_eq!(
        fixture_projection().tessellation_tolerance(0, &coarse).unwrap_err(),
        MvtError::BudgetBelowQuantization
    );
    assert_eq!(MvtEntity::new(eid(1), "", circle(0.0, 0.0, 1.0)).unwrap_err(), MvtError::EmptyLayerName);
    assert!(TileId::new(MAX_ZOOM, 1, 1).is_ok());
}

#[test]
fn issue_23_a_feature_smaller_than_a_tile_unit_is_unsupported_not_silently_dropped() {
    let tiny = Curve::Polyline(Polyline::new(vec![p(50.0, 50.0), p(50.0001, 50.0)]).unwrap());
    let ents = [MvtEntity::new(eid(3), "t", tiny).unwrap()];
    let pr = fixture_projection();
    // find the tile that holds it at zoom 0
    let t = export_tile(&ents, &pr, TileId::new(0, 0, 0).unwrap(), &MvtOptions::default()).unwrap();
    let recs = t.ledger.ledger().records();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].fidelity().kind(), FidelityKind::Unsupported);
    assert!(t.bytes.is_empty());
}

#[test]
fn issue_23_an_empty_tile_is_a_valid_empty_tile() {
    let pr = fixture_projection();
    let t = export_tile(&fixture(), &pr, TileId::new(14, 0, 0).unwrap(), &MvtOptions::default()).unwrap();
    assert!(t.bytes.is_empty());
    assert!(t.ledger.ledger().records().is_empty());
    assert!(decode_tile(&t.bytes, 64).is_empty());
}

// ---- the document -------------------------------------------------------------------------------------

fn doc() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/INTERCHANGE.md");
    fs::read_to_string(path).expect("docs/INTERCHANGE.md exists")
}

#[test]
fn issue_23_interchange_chapter_5_states_extent_zoom_range_and_quantization_error() {
    let d = doc();
    let ch = d.split("## 5. ").nth(1).expect("chapter 5 exists");
    assert!(ch.contains("display-only"));
    assert!(ch.contains(&format!("extent {DEFAULT_EXTENT}")));
    assert!(ch.contains(&format!("maximum tested zoom {MAX_TESTED_ZOOM}")));
    assert!(ch.contains("tested zoom range 0 to"));
    let pr = fixture_projection();
    let bound = format!("{:.3e}", pr.quantization_error(MAX_TESTED_ZOOM, DEFAULT_EXTENT));
    assert!(ch.contains(&bound), "chapter 5 must state the quantization error {bound}");
    let px = format!("{:.4}", pr.quantization_error_px(DEFAULT_EXTENT));
    assert!(ch.contains(&px), "chapter 5 must state the quantization error in pixels, {px}");
}
