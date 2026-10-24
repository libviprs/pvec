//! The GeoPackage export (issue P6.5, pvec#22): the file layout a GIS desktop needs to open it without
//! repair, the curve classes and what each claims, the measured deviation of every approximation, and
//! reproducibility. The file is read back by `common`, a SQLite reader that shares no code with the
//! writer; the independent GIS reader and the GeoPackage validator run from `tools/gpkg-validate.sh`.

mod common;

use common::*;
use pvec_capabilities::ids;
use pvec_entity::EntityId;
use pvec_geometry::*;
use pvec_interchange::gpkg::*;
use pvec_interchange::{Fidelity, FidelityKind, Representation};
use pvec_numeric::Policy;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI, TAU};

const POLICY: Policy = Policy::V1;

fn eid(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}
fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}
fn opts(tol: f64) -> GpkgOptions {
    GpkgOptions::new(tol).unwrap()
}
fn entity(n: u32, curve: Curve) -> GpkgEntity {
    GpkgEntity { entity_id: eid(n), curve }
}
fn circle(cx: f64, cy: f64, r: f64) -> Circle {
    Circle::new(Placement2::new(p(cx, cy), Vector2::new(1.0, 0.0)).unwrap(), r).unwrap()
}
fn arc(cx: f64, cy: f64, r: f64, t1: f64, t2: f64, sense: bool) -> Curve {
    Curve::Trimmed(Trimmed::new(Curve::Circle(circle(cx, cy, r)), t1, t2, sense).unwrap())
}
fn segment(a: Point2, b: Point2) -> Curve {
    let d = Vector2::new(b.x - a.x, b.y - a.y);
    let len = d.length();
    let line = Line::new(a, Vector2::new(d.x / len, d.y / len)).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, len, true).unwrap())
}
fn polyline(v: &[(f64, f64)]) -> Curve {
    Curve::Polyline(Polyline::new(v.iter().map(|&(x, y)| p(x, y)).collect()).unwrap())
}
fn ellipse(a: f64, ratio: f64) -> Curve {
    Curve::Ellipse(Ellipse::from_major_axis(p(10.0, -5.0), Vector2::new(a, 0.0), ratio).unwrap())
}
fn cubic_bezier() -> Curve {
    Curve::Bezier(Bezier::new(vec![p(0.0, 0.0), p(2.0, 5.0), p(7.0, 5.0), p(9.0, 0.0)]).unwrap())
}
fn cubic_bspline() -> Curve {
    let pts = vec![p(0.0, 0.0), p(1.0, 3.0), p(3.0, 4.0), p(5.0, 1.0), p(7.0, 3.0), p(9.0, 0.0)];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0];
    Curve::BSpline(BSpline::new(3, pts, knots).unwrap())
}
/// A rational quadratic that is a hyperbola arc, not a circle.
fn hyperbola() -> Curve {
    Curve::RationalBSpline(
        RationalBSpline::new(
            2,
            vec![p(0.0, 0.0), p(5.0, 8.0), p(10.0, 0.0)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![1.0, 3.0, 1.0],
        )
        .unwrap(),
    )
}
/// The nine control point rational quadratic circle.
fn nurbs_circle(cx: f64, cy: f64, r: f64) -> Curve {
    let w = FRAC_1_SQRT_2;
    let pts = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let weights = vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0];
    let knots = vec![0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0];
    let pts = pts.iter().map(|&(x, y)| p(cx + r * x, cy + r * y)).collect();
    Curve::RationalBSpline(RationalBSpline::new(2, pts, knots, weights).unwrap())
}
fn linear_rational() -> Curve {
    Curve::RationalBSpline(
        RationalBSpline::new(
            1,
            vec![p(0.0, 0.0), p(4.0, 3.0), p(8.0, 0.0)],
            vec![0.0, 0.0, 1.0, 2.0, 2.0],
            vec![1.0, 2.0, 5.0],
        )
        .unwrap(),
    )
}

fn open(bytes: &[u8]) -> Db {
    Db::open(bytes.to_vec())
}
fn features(db: &Db) -> Vec<BTreeMap<String, Value>> {
    db.named_rows(TABLE_NAME)
}
fn geom_of(row: &BTreeMap<String, Value>) -> GpkgGeom {
    decode_gpkg(row["geom"].blob())
}
fn export_one(c: Curve, tol: f64) -> (GpkgExport, Db) {
    let e = export(&[entity(1, c)], &opts(tol)).unwrap();
    let db = open(&e.bytes);
    (e, db)
}
fn arc_points(w: &Wkb) -> &[[f64; 2]] {
    match w {
        Wkb::CircularString(p) => p,
        other => panic!("not a CIRCULARSTRING: {other:?}"),
    }
}

/// Largest distance from the exact curve to the polyline, and from the polyline to the exact curve,
/// both sampled far more finely than the writer measures.
fn independent_deviation(curve: &Curve, poly: &[[f64; 2]]) -> f64 {
    let d = curve.domain();
    let n = 6000;
    let dense: Vec<[f64; 2]> = (0..=n)
        .map(|i| {
            let t = d.lo() + d.width() * i as f64 / n as f64;
            let q = curve.eval(t.clamp(d.lo(), d.hi())).unwrap();
            [q.x, q.y]
        })
        .collect();
    let seg = |a: [f64; 2], b: [f64; 2], q: [f64; 2]| -> f64 {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t =
            if l2 == 0.0 { 0.0 } else { (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
        (q[0] - a[0] - t * dx).hypot(q[1] - a[1] - t * dy)
    };
    let mut worst: f64 = 0.0;
    for q in &dense {
        let m = poly.windows(2).map(|w| seg(w[0], w[1], *q)).fold(f64::INFINITY, f64::min);
        worst = worst.max(m);
    }
    for w in poly.windows(2) {
        for k in 0..=16 {
            let f = k as f64 / 16.0;
            let q = [w[0][0] + f * (w[1][0] - w[0][0]), w[0][1] + f * (w[1][1] - w[0][1])];
            let m = dense.windows(2).map(|s| seg(s[0], s[1], q)).fold(f64::INFINITY, f64::min);
            worst = worst.max(m);
        }
    }
    worst
}

fn line_points(w: &Wkb) -> &[[f64; 2]] {
    match w {
        Wkb::LineString(p) => p,
        other => panic!("not a LINESTRING: {other:?}"),
    }
}

// ---- the file opens without repair ------------------------------------------------------------------

#[test]
fn the_header_carries_the_recorded_write_settings() {
    let (e, db) = export_one(polyline(&[(0.0, 0.0), (1.0, 1.0)]), 0.01);
    let s = WRITE_SETTINGS;
    assert_eq!(db.page_size, s.page_size as usize);
    assert_eq!(db.header_u32(68), 0x4750_4B47, "application_id is GPKG");
    assert_eq!(s.application_id, 0x4750_4B47);
    assert_eq!(db.header_u32(60), 10400, "user_version is 1.4.0");
    assert_eq!(db.header_u32(44), s.schema_format);
    assert_eq!(db.header_u32(56), s.text_encoding);
    assert_eq!(db.header_u32(96), s.sqlite_version_number);
    assert_eq!(e.bytes[18], 1, "rollback journal, not WAL");
    assert_eq!(e.bytes[19], 1);
    assert_eq!(db.header_u32(24), db.header_u32(92), "change counter equals version-valid-for");
}

#[test]
fn the_core_tables_exist_with_the_required_rows() {
    let (_, db) = export_one(polyline(&[(0.0, 0.0), (1.0, 1.0)]), 0.01);
    let srs = db.named_rows("gpkg_spatial_ref_sys");
    let ids: Vec<i64> = srs.iter().map(|r| r["srs_id"].int()).collect();
    for needed in [-1, 0, 4326] {
        assert!(ids.contains(&needed), "gpkg_spatial_ref_sys lacks srs_id {needed}");
    }
    let wgs = srs.iter().find(|r| r["srs_id"].int() == 4326).unwrap();
    assert_eq!(wgs["organization"].text(), "EPSG");
    assert_eq!(wgs["organization_coordsys_id"].int(), 4326);
    let contents = db.named_rows("gpkg_contents");
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0]["table_name"].text(), TABLE_NAME);
    assert_eq!(contents[0]["data_type"].text(), "features");
    assert_eq!(contents[0]["srs_id"].int(), -1, "a drawing is Cartesian, not geographic");
    assert_eq!(contents[0]["last_change"].text(), DEFAULT_LAST_CHANGE);
    let gc = db.named_rows("gpkg_geometry_columns");
    assert_eq!(gc.len(), 1);
    assert_eq!(gc[0]["table_name"].text(), TABLE_NAME);
    assert_eq!(gc[0]["column_name"].text(), GEOMETRY_COLUMN);
    assert_eq!(gc[0]["geometry_type_name"].text(), "GEOMETRY");
    assert_eq!((gc[0]["z"].int(), gc[0]["m"].int()), (0, 0));
    db.named_rows("gpkg_extensions");
}

#[test]
fn the_indexes_behind_the_unique_constraints_exist_and_agree_with_the_tables() {
    let (_, db) = export_one(arc(0.0, 0.0, 5.0, 0.0, PI, true), 0.01);
    let master = db.master();
    for (table, expect) in [("gpkg_contents", 2), ("gpkg_geometry_columns", 2), ("gpkg_extensions", 1)] {
        let autos: Vec<&MasterRow> =
            master.iter().filter(|m| m.kind == "index" && m.table == table && m.sql.is_none()).collect();
        assert_eq!(autos.len(), expect, "{table}: autoindexes");
        for a in autos {
            assert!(a.name.starts_with(&format!("sqlite_autoindex_{table}_")), "{}", a.name);
            let entries = db.index_entries(a.root);
            assert_eq!(entries.len(), db.rows(table).len(), "{table}: one index entry per row");
        }
    }
}

#[test]
fn the_feature_table_has_the_attribute_columns_a_desktop_user_reads() {
    let (_, db) = export_one(cubic_bezier(), 0.01);
    let sql = db.find(TABLE_NAME).sql.unwrap();
    let cols = column_names(&sql);
    assert_eq!(cols, ["fid", "geom", "entity_id", "fidelity", "error_bound", "source_capability"]);
    assert!(sql.contains("\"fid\" INTEGER PRIMARY KEY"));
}

#[test]
fn every_row_carries_the_entity_id_and_the_fidelity_value() {
    let ents = vec![
        entity(7, polyline(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)])),
        entity(3, arc(0.0, 0.0, 5.0, 0.2, 2.0, true)),
        entity(11, cubic_bezier()),
        entity(2, ellipse(6.0, 0.5)),
    ];
    let e = export(&ents, &opts(0.01)).unwrap();
    let db = open(&e.bytes);
    let rows = features(&db);
    assert_eq!(rows.len(), 4);
    let got: Vec<(i64, i64, &str)> =
        rows.iter().map(|r| (r["fid"].int(), r["entity_id"].int(), r["fidelity"].text())).collect();
    assert_eq!(
        got,
        vec![
            (1, 7, "Exact"),
            (2, 3, "ExactReparameterized"),
            (3, 11, "BoundedApproximation"),
            (4, 2, "BoundedApproximation")
        ]
    );
    assert!(rows[0]["error_bound"].is_null() && rows[1]["error_bound"].is_null());
    assert!(rows[2]["error_bound"].real() > 0.0 && rows[3]["error_bound"].real() > 0.0);
    assert_eq!(rows[0]["source_capability"].int(), i64::from(ids::POLYLINE.get()));
    assert_eq!(rows[1]["source_capability"].int(), i64::from(ids::TRIMMED_CURVE.get()));
    assert_eq!(rows[2]["source_capability"].int(), i64::from(ids::BEZIER_CURVE.get()));
    assert_eq!(rows[3]["source_capability"].int(), i64::from(ids::ELLIPSE.get()));
}

#[test]
fn the_contents_bounds_are_the_union_of_the_feature_envelopes() {
    let ents =
        vec![entity(1, arc(0.0, 0.0, 5.0, 0.0, PI, true)), entity(2, polyline(&[(-9.0, -2.0), (3.0, 7.0)]))];
    let e = export(&ents, &opts(0.01)).unwrap();
    let db = open(&e.bytes);
    let c = &db.named_rows("gpkg_contents")[0];
    assert!(POLICY.near(c["min_x"].real(), -9.0, 14.0), "{:?}", c["min_x"]);
    assert!(POLICY.near(c["max_x"].real(), 5.0, 14.0));
    assert!(POLICY.near(c["min_y"].real(), -2.0, 14.0));
    assert!(POLICY.near(c["max_y"].real(), 7.0, 14.0));
}

// ---- the extension rows ---------------------------------------------------------------------------

fn extension_names(db: &Db) -> Vec<(String, String, String, String)> {
    db.named_rows("gpkg_extensions")
        .iter()
        .map(|r| {
            (
                r["table_name"].text().to_string(),
                match &r["column_name"] {
                    Value::Text(s) => s.clone(),
                    _ => String::new(),
                },
                r["extension_name"].text().to_string(),
                r["scope"].text().to_string(),
            )
        })
        .collect()
}

#[test]
fn a_nonlinear_type_in_the_file_has_its_extension_row_and_an_unused_type_has_none() {
    let ents =
        vec![entity(1, arc(0.0, 0.0, 5.0, 0.0, 1.0, true)), entity(2, polyline(&[(0.0, 0.0), (1.0, 2.0)]))];
    let e = export(&ents, &opts(0.01)).unwrap();
    let db = open(&e.bytes);
    let ext = extension_names(&db);
    let named = |n: &str| ext.iter().filter(|x| x.2 == n).count();
    assert_eq!(named("gpkg_geom_CIRCULARSTRING"), 1, "{ext:?}");
    assert_eq!(named("gpkg_geom_COMPOUNDCURVE"), 0, "a type nothing uses gets no row");
    assert_eq!(named("gpkg_geom_CURVEPOLYGON"), 0);
    let row = ext.iter().find(|x| x.2 == "gpkg_geom_CIRCULARSTRING").unwrap();
    assert_eq!((row.0.as_str(), row.1.as_str(), row.3.as_str()), (TABLE_NAME, GEOMETRY_COLUMN, "read-write"));
    let defn = db.named_rows("gpkg_extensions");
    let d = defn.iter().find(|r| r["extension_name"].text() == "gpkg_geom_CIRCULARSTRING").unwrap();
    assert!(d["definition"].text().contains("geopackage.org"));

    let lines_only = export(&[entity(1, polyline(&[(0.0, 0.0), (1.0, 2.0)]))], &opts(0.01)).unwrap();
    let ext = extension_names(&open(&lines_only.bytes));
    assert!(ext.iter().all(|x| !x.2.starts_with("gpkg_geom_")), "{ext:?}");
    assert!(ext.iter().any(|x| x.2 == "gpkg_rtree_index"), "the spatial index is an extension too");
}

#[test]
fn a_compound_curve_registers_both_nonlinear_extensions_it_uses() {
    let comp = Composite::new(vec![
        segment(p(0.0, 0.0), p(4.0, 0.0)),
        arc(4.0, 3.0, 3.0, 3.0 * FRAC_PI_2, TAU, true),
    ]);
    let comp = Curve::Composite(comp.unwrap());
    let (_, db) = export_one(comp, 0.01);
    let ext = extension_names(&db);
    assert!(ext.iter().any(|x| x.2 == "gpkg_geom_COMPOUNDCURVE"), "{ext:?}");
    assert!(ext.iter().any(|x| x.2 == "gpkg_geom_CIRCULARSTRING"), "{ext:?}");
}

// ---- exact classes -------------------------------------------------------------------------------

#[test]
fn a_polyline_is_a_linestring_with_the_same_vertices_bit_for_bit() {
    let v = [(0.1, 0.2), (3.3, 4.4), (-7.7, 2.2), (100000.3, -100000.0)];
    let (e, db) = export_one(polyline(&v), 0.01);
    let g = geom_of(&features(&db)[0]);
    let pts = line_points(&g.wkb);
    assert_eq!(pts.len(), 4);
    for (a, b) in pts.iter().zip(v) {
        assert_eq!((a[0].to_bits(), a[1].to_bits()), (b.0.to_bits(), b.1.to_bits()));
    }
    let r = &e.ledger.records()[0];
    assert_eq!(r.fidelity().kind(), FidelityKind::Exact);
    assert_eq!(r.target_construct().name(), "LINESTRING");
    assert_eq!(r.target_construct().representation(), Representation::Native);
    assert_eq!(r.source_capability(), ids::POLYLINE);
}

#[test]
fn a_trimmed_line_is_a_two_point_linestring_ending_where_the_segment_ends() {
    let (e, db) = export_one(segment(p(1.0, 1.0), p(4.0, 5.0)), 0.01);
    let g = geom_of(&features(&db)[0]);
    let pts = line_points(&g.wkb);
    assert_eq!(pts.len(), 2);
    assert!(POLICY.near_point(pts[0], [1.0, 1.0], 5.0));
    assert!(POLICY.near_point(pts[1], [4.0, 5.0], 5.0));
    assert_eq!(e.ledger.records()[0].fidelity().kind(), FidelityKind::ExactReparameterized);
}

#[test]
fn an_arc_round_trips_through_circularstring_with_its_centre_radius_and_end_points() {
    for (cx, cy, r, t1, t2, sense) in [
        (3.0, -2.0, 5.0, 0.3, 1.9, true),
        (-40.0, 17.5, 123.25, 4.0, 5.5, true),
        (0.0, 0.0, 1.0, 2.9, 0.1, false),
        (10.0, 10.0, 2.5, 5.9, 0.4, true),
    ] {
        let curve = arc(cx, cy, r, t1, t2, sense);
        let (e, db) = export_one(curve.clone(), 0.01);
        let g = geom_of(&features(&db)[0]);
        assert_eq!(g.srs_id, -1);
        let pts = arc_points(&g.wkb);
        assert_eq!(pts.len(), 3, "an arc of at most half a turn is one three-point arc");
        let (centre, radius) = circumcircle(pts[0], pts[1], pts[2]).unwrap();
        assert!(POLICY.near_point(centre, [cx, cy], r), "centre {centre:?} vs ({cx},{cy})");
        assert!(POLICY.near(radius, r, r), "radius {radius} vs {r}");
        let d = curve.domain();
        let (s, t) = (curve.eval(d.lo()).unwrap(), curve.eval(d.hi()).unwrap());
        assert!(POLICY.near_point(pts[0], [s.x, s.y], r), "start");
        assert!(POLICY.near_point(pts[2], [t.x, t.y], r), "end");
        let m = curve.eval(0.5 * (d.lo() + d.hi())).unwrap();
        assert!(POLICY.near_point(pts[1], [m.x, m.y], r), "the middle point is the curve's own middle");
        let rec = &e.ledger.records()[0];
        assert_eq!(rec.fidelity().kind(), FidelityKind::ExactReparameterized);
        assert_eq!(rec.target_construct().name(), "CIRCULARSTRING");
        assert_eq!(rec.target_construct().representation(), Representation::Native);
    }
}

#[test]
fn an_arc_keeps_its_direction() {
    let ccw = geom_of(&features(&export_one(arc(0.0, 0.0, 5.0, 0.2, 1.4, true), 0.01).1)[0]);
    let cw = geom_of(&features(&export_one(arc(0.0, 0.0, 5.0, 1.4, 0.2, false), 0.01).1)[0]);
    let (a, b) = (arc_points(&ccw.wkb), arc_points(&cw.wkb));
    assert!(POLICY.near_point(a[0], b[2], 5.0) && POLICY.near_point(a[2], b[0], 5.0), "reversed ends");
    assert!(POLICY.near_point(a[1], b[1], 5.0));
}

#[test]
fn an_arc_of_more_than_half_a_turn_stays_one_circle() {
    let curve = arc(2.0, 3.0, 4.0, 0.5, 5.5, true);
    let (_, db) = export_one(curve.clone(), 0.01);
    let g = geom_of(&features(&db)[0]);
    let pts = arc_points(&g.wkb);
    assert_eq!(pts.len(), 5, "two three-point arcs of one circle, which the format spells as five points");
    for q in pts {
        assert!(POLICY.near((q[0] - 2.0).hypot(q[1] - 3.0), 4.0, 4.0));
    }
    let d = curve.domain();
    let (s, t) = (curve.eval(d.lo()).unwrap(), curve.eval(d.hi()).unwrap());
    assert!(POLICY.near_point(pts[0], [s.x, s.y], 4.0) && POLICY.near_point(pts[4], [t.x, t.y], 4.0));
}

#[test]
fn a_whole_circle_is_a_closed_circularstring() {
    let (e, db) = export_one(Curve::Circle(circle(1.0, 2.0, 3.0)), 0.01);
    let g = geom_of(&features(&db)[0]);
    let pts = arc_points(&g.wkb);
    assert_eq!(pts.len(), 5);
    assert_eq!(pts[0], pts[4], "closed exactly");
    for q in pts {
        assert!(POLICY.near((q[0] - 1.0).hypot(q[1] - 2.0), 3.0, 3.0));
    }
    assert_eq!(e.ledger.records()[0].fidelity().kind(), FidelityKind::ExactReparameterized);
    for (got, want) in g.envelope.iter().zip([-2.0, 4.0, -1.0, 5.0]) {
        assert!(POLICY.near(*got, want, 3.0), "the envelope of a whole circle: {:?}", g.envelope);
    }
}

#[test]
fn the_envelope_of_an_arc_covers_its_extremes_not_just_its_end_points() {
    // from 30 degrees to 150 degrees the top of the circle is not an end point
    let curve = arc(0.0, 0.0, 10.0, PI / 6.0, 5.0 * PI / 6.0, true);
    let (_, db) = export_one(curve, 0.01);
    let g = geom_of(&features(&db)[0]);
    assert!(POLICY.near(g.envelope[3], 10.0, 10.0), "max y is the top of the circle: {:?}", g.envelope);
    assert!(POLICY.near(g.envelope[2], 5.0, 10.0), "min y is the end points' height: {:?}", g.envelope);
    let (slack, reach) = (POLICY.tol(10.0, 10.0), 10.0 * (PI / 6.0).cos());
    assert!(g.envelope[0] <= -reach + slack && g.envelope[1] >= reach - slack);
}

// ---- approximations carry a measured bound ----------------------------------------------------------

#[test]
fn every_approximated_entity_has_a_measured_deviation_at_or_below_the_request_and_the_ledger_records_it() {
    let curves = [
        ("ellipse", ellipse(6.0, 0.4)),
        ("partial ellipse", Curve::Trimmed(Trimmed::new(ellipse(6.0, 0.4), 0.3, 4.0, true).unwrap())),
        ("cubic bezier", cubic_bezier()),
        ("cubic b-spline", cubic_bspline()),
        ("hyperbola", hyperbola()),
    ];
    for tol in [0.1, 0.01, 0.0005] {
        for (name, curve) in &curves {
            let (e, db) = export_one(curve.clone(), tol);
            let rec = &e.ledger.records()[0];
            let bound =
                rec.error_bound().unwrap_or_else(|| panic!("{name}: no bound ({:?})", rec.fidelity())).get();
            assert!(bound > 0.0 && bound <= tol, "{name}@{tol}: recorded {bound}");
            assert!(bound < tol, "{name}@{tol}: the ledger holds the measured value, not the request");
            let row = &features(&db)[0];
            assert_eq!(row["error_bound"].real(), bound, "{name}: the column and the ledger agree");
            assert_eq!(row["fidelity"].text(), "BoundedApproximation");
            let g = geom_of(row);
            let independent = independent_deviation(curve, line_points(&g.wkb));
            assert!(independent <= tol, "{name}@{tol}: exact evaluation says {independent} > tolerance");
            assert!(
                bound >= independent * 0.99,
                "{name}@{tol}: recorded {bound} is below what exact evaluation finds ({independent})"
            );
            assert_eq!(rec.target_construct().name(), "LINESTRING");
            assert_eq!(rec.target_construct().representation(), Representation::Tessellated);
        }
    }
}

#[test]
fn a_tighter_tolerance_gives_more_vertices_and_a_smaller_measured_value() {
    let count = |tol: f64| {
        let (e, db) = export_one(ellipse(6.0, 0.4), tol);
        let n = line_points(&geom_of(&features(&db)[0]).wkb).len();
        (n, e.ledger.records()[0].error_bound().unwrap().get())
    };
    let (n1, b1) = count(0.1);
    let (n2, b2) = count(0.001);
    assert!(n2 > n1 && b2 < b1, "{n1}/{b1} then {n2}/{b2}");
}

#[test]
fn a_tolerance_the_coordinates_cannot_resolve_is_refused_not_looped_on() {
    let big = Curve::Circle(circle(1.0e8, 1.0e8, 1.0e6));
    let e = Curve::Ellipse(Ellipse::from_major_axis(p(1.0e8, 1.0e8), Vector2::new(1.0e6, 0.0), 0.5).unwrap());
    let _ = big;
    assert!(matches!(translate(&e, 0.000000000001), Err(GpkgError::ToleranceUnreachable { .. })));
}

#[test]
fn the_tolerance_is_checked_when_the_options_are_built() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(GpkgOptions::new(bad), Err(GpkgError::BadTolerance)), "{bad}");
    }
    assert!(GpkgOptions::new(0.000001).is_ok());
}

#[test]
fn the_last_change_stamp_must_be_an_iso_timestamp() {
    assert!(opts(0.1).with_last_change("2026-10-24T12:00:00.000Z").is_ok());
    for bad in ["", "yesterday", "2026-10-24", "2026-10-24 12:00:00", "2026-13-40T99:99:99.000Z"] {
        assert!(matches!(opts(0.1).with_last_change(bad), Err(GpkgError::BadTimestamp)), "{bad}");
    }
    let o = opts(0.1).with_last_change("2026-10-24T12:00:00.000Z").unwrap();
    let e = export(&[entity(1, polyline(&[(0.0, 0.0), (1.0, 1.0)]))], &o).unwrap();
    assert_eq!(
        open(&e.bytes).named_rows("gpkg_contents")[0]["last_change"].text(),
        "2026-10-24T12:00:00.000Z"
    );
}

// ---- rational B-splines are never exact -------------------------------------------------------------

#[test]
fn a_rational_b_spline_that_is_not_made_of_arcs_is_a_bounded_approximation_with_a_bound() {
    let t = translate(&hyperbola(), 0.01).unwrap();
    match t.fidelity {
        Fidelity::BoundedApproximation(b) => assert!(b.get() > 0.0 && b.get() <= 0.01),
        other => panic!("{other:?}"),
    }
    assert_eq!(t.source_capability, ids::RATIONAL_B_SPLINE_CURVE);
}

#[test]
fn no_rational_b_spline_is_ever_reported_exact_in_either_exact_class() {
    for (name, c) in [
        ("hyperbola", hyperbola()),
        ("nurbs circle", nurbs_circle(0.0, 0.0, 5.0)),
        ("linear", linear_rational()),
    ] {
        let k = translate(&c, 0.01).unwrap().fidelity.kind();
        assert!(
            matches!(k, FidelityKind::EquivalentCompound | FidelityKind::BoundedApproximation),
            "{name} came out {k:?}"
        );
    }
}

#[test]
fn a_rational_circle_made_of_arc_spans_is_an_equivalent_compound_of_circular_arcs() {
    let curve = nurbs_circle(2.0, -1.0, 5.0);
    let (e, db) = export_one(curve.clone(), 0.01);
    let rec = &e.ledger.records()[0];
    assert_eq!(rec.fidelity().kind(), FidelityKind::EquivalentCompound);
    assert_eq!(rec.target_construct().representation(), Representation::Native);
    assert_eq!(rec.target_construct().name(), "CIRCULARSTRING");
    let g = geom_of(&features(&db)[0]);
    let pts = arc_points(&g.wkb);
    assert_eq!(pts.len(), 9, "four arcs of a quarter turn");
    for arc in pts.windows(3).step_by(2) {
        let (c, r) = circumcircle(arc[0], arc[1], arc[2]).unwrap();
        assert!(POLICY.near_point(c, [2.0, -1.0], 5.0) && POLICY.near(r, 5.0, 5.0), "{c:?} {r}");
    }
    // and the locus is the source locus, by exact evaluation
    let d = curve.domain();
    for k in 0..=200 {
        let q = curve.eval(d.lo() + d.width() * k as f64 / 200.0).unwrap();
        assert!(POLICY.near((q.x - 2.0).hypot(q.y + 1.0), 5.0, 5.0));
    }
}

#[test]
fn a_degree_one_spline_is_the_polyline_through_its_knots_and_an_equivalent_compound() {
    let (e, db) = export_one(linear_rational(), 0.01);
    assert_eq!(e.ledger.records()[0].fidelity().kind(), FidelityKind::EquivalentCompound);
    let g = geom_of(&features(&db)[0]);
    let pts = line_points(&g.wkb);
    assert_eq!(pts.len(), 3);
    assert!(POLICY.near_point(pts[0], [0.0, 0.0], 8.0) && POLICY.near_point(pts[1], [4.0, 3.0], 8.0));
    assert!(POLICY.near_point(pts[2], [8.0, 0.0], 8.0));
    assert!(e.ledger.records()[0].error_bound().is_none());
}

#[test]
fn a_tessellated_target_is_never_recorded_as_one_of_the_exact_values() {
    for c in [ellipse(5.0, 0.5), cubic_bezier(), cubic_bspline(), hyperbola()] {
        let (e, _) = export_one(c, 0.05);
        let r = &e.ledger.records()[0];
        assert_eq!(r.target_construct().representation(), Representation::Tessellated);
        assert_eq!(r.fidelity().kind(), FidelityKind::BoundedApproximation);
    }
}

// ---- composite curves ---------------------------------------------------------------------------

#[test]
fn a_composite_of_lines_and_arcs_is_a_compound_curve_whose_pieces_join_exactly() {
    let comp = Composite::new(vec![
        segment(p(0.0, 0.0), p(4.0, 0.0)),
        arc(4.0, 3.0, 3.0, -FRAC_PI_2, 0.0, true),
        polyline(&[(7.0, 3.0), (7.0, 8.0), (2.0, 8.0)]),
    ])
    .unwrap();
    let (e, db) = export_one(Curve::Composite(comp), 0.01);
    let g = geom_of(&features(&db)[0]);
    let Wkb::CompoundCurve(parts) = &g.wkb else { panic!("{:?}", g.wkb) };
    assert_eq!(parts.len(), 3);
    assert!(matches!(parts[0], Wkb::LineString(_)) && matches!(parts[1], Wkb::CircularString(_)));
    let ends = |w: &Wkb| match w {
        Wkb::LineString(p) | Wkb::CircularString(p) => (p[0], *p.last().unwrap()),
        _ => unreachable!(),
    };
    for pair in parts.windows(2) {
        assert_eq!(ends(&pair[0]).1, ends(&pair[1]).0, "a compound curve's pieces must be connected exactly");
    }
    let rec = &e.ledger.records()[0];
    assert_eq!(rec.target_construct().name(), "COMPOUNDCURVE");
    assert_eq!(rec.fidelity().kind(), FidelityKind::ExactReparameterized, "the weakest exact piece decides");
    assert_eq!(rec.source_capability(), ids::COMPOSITE_CURVE);
}

#[test]
fn a_composite_with_a_spline_piece_is_bounded_by_the_worst_piece_and_still_joins_exactly() {
    let comp = Composite::new(vec![
        segment(p(-3.0, 0.0), p(0.0, 0.0)),
        cubic_bezier(),
        segment(p(9.0, 0.0), p(12.0, 4.0)),
    ])
    .unwrap();
    let (e, db) = export_one(Curve::Composite(comp.clone()), 0.01);
    let rec = &e.ledger.records()[0];
    let bound = rec.error_bound().expect("a bounded approximation").get();
    assert!(bound > 0.0 && bound <= 0.01);
    let g = geom_of(&features(&db)[0]);
    let Wkb::CompoundCurve(parts) = &g.wkb else { panic!() };
    let ends = |w: &Wkb| match w {
        Wkb::LineString(p) | Wkb::CircularString(p) => (p[0], *p.last().unwrap()),
        _ => unreachable!(),
    };
    for pair in parts.windows(2) {
        assert_eq!(ends(&pair[0]).1, ends(&pair[1]).0);
    }
    // the whole written chain stays within the tolerance of the exact composite
    let mut all: Vec<[f64; 2]> = Vec::new();
    for w in parts {
        all.extend_from_slice(line_points(w));
    }
    all.dedup();
    assert!(independent_deviation(&Curve::Composite(comp), &all) <= 0.01);
}

// ---- unsupported and refused ----------------------------------------------------------------------

#[test]
fn an_unbounded_line_is_unsupported_with_a_reason_and_writes_no_row() {
    let line = Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap());
    let ents =
        vec![entity(1, polyline(&[(0.0, 0.0), (1.0, 1.0)])), entity(2, line), entity(3, cubic_bezier())];
    let e = export(&ents, &opts(0.01)).unwrap();
    let db = open(&e.bytes);
    let rows = features(&db);
    assert_eq!(rows.iter().map(|r| r["entity_id"].int()).collect::<Vec<_>>(), vec![1, 3]);
    assert_eq!(e.ledger.records().len(), 3, "every entity is accounted for in the ledger");
    let u = &e.ledger.records()[1];
    match u.fidelity() {
        Fidelity::Unsupported(reason) => assert!(reason.as_str().contains("unbounded")),
        other => panic!("{other:?}"),
    }
    assert_eq!(u.entity_id(), eid(2));
}

#[test]
fn a_repeated_entity_id_is_refused() {
    let ents = vec![entity(4, cubic_bezier()), entity(4, cubic_bezier())];
    assert!(matches!(export(&ents, &opts(0.1)), Err(GpkgError::DuplicateEntity(id)) if id == eid(4)));
}

#[test]
fn an_export_of_no_entities_is_a_valid_empty_geopackage() {
    let e = export(&[], &opts(0.1)).unwrap();
    let db = open(&e.bytes);
    assert!(features(&db).is_empty());
    assert!(e.ledger.records().is_empty());
    let c = &db.named_rows("gpkg_contents")[0];
    assert!(c["min_x"].is_null() && c["max_y"].is_null());
}

// ---- the spatial index ------------------------------------------------------------------------------

fn f32_at(b: &[u8], at: usize) -> f64 {
    f64::from(f32::from_be_bytes(b[at..at + 4].try_into().unwrap()))
}

struct Node {
    cells: Vec<(i64, [f64; 4])>,
}

fn read_node(data: &[u8]) -> Node {
    let n = u16::from_be_bytes([data[2], data[3]]) as usize;
    let cells = (0..n)
        .map(|i| {
            let at = 4 + i * 24;
            let id = i64::from_be_bytes(data[at..at + 8].try_into().unwrap());
            (id, [f32_at(data, at + 8), f32_at(data, at + 12), f32_at(data, at + 16), f32_at(data, at + 20)])
        })
        .collect();
    Node { cells }
}

fn many_entities(n: u32) -> Vec<GpkgEntity> {
    (1..=n)
        .map(|i| {
            let x = f64::from(i % 97) * 3.0;
            let y = f64::from(i / 97) * 3.0;
            if i % 5 == 0 {
                entity(i, arc(x, y, 1.0, 0.0, 2.0, true))
            } else {
                entity(i, polyline(&[(x, y), (x + 1.5, y + 0.5)]))
            }
        })
        .collect()
}

#[test]
fn the_spatial_index_is_populated_and_every_leaf_box_contains_its_feature() {
    let ents = many_entities(1500);
    let e = export(&ents, &opts(0.01)).unwrap();
    let db = open(&e.bytes);
    let rt = "rtree_pvec_features_geom";
    let master = db.master();
    let virt = master.iter().find(|m| m.name == rt).expect("the virtual table");
    assert_eq!(virt.root, 0);
    assert!(virt.sql.as_ref().unwrap().contains("USING rtree(id, minx, maxx, miny, maxy)"));
    for shadow in ["_node", "_parent", "_rowid"] {
        assert!(master.iter().any(|m| m.name == format!("{rt}{shadow}") && m.kind == "table"), "{shadow}");
    }
    let nodes: BTreeMap<i64, Vec<u8>> =
        db.rows(&format!("{rt}_node")).into_iter().map(|r| (r.rowid, r.values[1].blob().to_vec())).collect();
    let page_node_size = db.page_size - 64;
    for (n, data) in &nodes {
        assert_eq!(
            data.len(),
            page_node_size,
            "node {n} must be exactly the node size SQLite's rtree expects"
        );
    }
    let root = &nodes[&1];
    let depth = u16::from_be_bytes([root[0], root[1]]) as usize;
    assert!(depth >= 1, "1500 entries need more than one level");
    let parents: BTreeMap<i64, i64> =
        db.rows(&format!("{rt}_parent")).into_iter().map(|r| (r.rowid, r.values[1].int())).collect();
    let rowids: BTreeMap<i64, i64> =
        db.rows(&format!("{rt}_rowid")).into_iter().map(|r| (r.rowid, r.values[1].int())).collect();
    assert_eq!(rowids.len(), 1500, "every feature is reachable through the rowid table");
    // walk from the root: interior cells name child nodes whose parent entry points back
    let mut seen = BTreeMap::new();
    fn walk(
        n: i64,
        level: usize,
        nodes: &BTreeMap<i64, Vec<u8>>,
        parents: &BTreeMap<i64, i64>,
        rowids: &BTreeMap<i64, i64>,
        seen: &mut BTreeMap<i64, [f64; 4]>,
    ) -> [f64; 4] {
        let node = read_node(&nodes[&n]);
        let mut hull = [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY];
        assert!(!node.cells.is_empty() || n == 1);
        for (id, b) in node.cells {
            let b = if level == 0 {
                assert_eq!(rowids[&id], n, "the rowid table names the leaf holding feature {id}");
                seen.insert(id, b);
                b
            } else {
                assert_eq!(parents[&id], n, "node {id}'s parent entry");
                let inner = walk(id, level - 1, nodes, parents, rowids, seen);
                assert!(
                    b[0] <= inner[0] && b[1] >= inner[1] && b[2] <= inner[2] && b[3] >= inner[3],
                    "interior box covers its child"
                );
                b
            };
            hull = [hull[0].min(b[0]), hull[1].max(b[1]), hull[2].min(b[2]), hull[3].max(b[3])];
        }
        hull
    }
    walk(1, depth, &nodes, &parents, &rowids, &mut seen);
    assert_eq!(seen.len(), 1500);
    for row in features(&db) {
        let g = geom_of(&row);
        let b = seen[&row["fid"].int()];
        let e = g.envelope;
        assert!(
            b[0] <= e[0] && b[1] >= e[1] && b[2] <= e[2] && b[3] >= e[3],
            "feature {} box {b:?} vs {e:?}",
            row["fid"].int()
        );
    }
}

#[test]
fn the_seven_spatial_index_triggers_of_geopackage_1_4_are_in_the_schema() {
    let (_, db) = export_one(cubic_bezier(), 0.1);
    let names: Vec<String> =
        db.master().into_iter().filter(|m| m.kind == "trigger").map(|m| m.name).collect();
    for suffix in ["insert", "update2", "update4", "update5", "update6", "update7", "delete"] {
        let n = format!("rtree_{TABLE_NAME}_{GEOMETRY_COLUMN}_{suffix}");
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    let ext = extension_names(&db);
    let row = ext.iter().find(|x| x.2 == "gpkg_rtree_index").unwrap();
    assert_eq!((row.0.as_str(), row.1.as_str(), row.3.as_str()), (TABLE_NAME, GEOMETRY_COLUMN, "write-only"));
}

#[test]
fn a_file_larger_than_one_page_of_schema_and_a_big_geometry_still_reads_back() {
    // a polyline of 5000 vertices overflows a page; the schema alone spans several
    let v: Vec<(f64, f64)> = (0..5000).map(|i| (f64::from(i), f64::from(i % 7))).collect();
    let (_, db) = export_one(polyline(&v), 0.1);
    let g = geom_of(&features(&db)[0]);
    assert_eq!(line_points(&g.wkb).len(), 5000);
    assert!(db.master().len() > 15);
}

// ---- reproducibility ---------------------------------------------------------------------------------

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn sample() -> Vec<GpkgEntity> {
    vec![
        entity(5, polyline(&[(0.0, 0.0), (2.0, 1.0), (4.0, -1.0)])),
        entity(2, arc(3.0, 3.0, 2.0, 0.1, 2.0, true)),
        entity(9, cubic_bezier()),
        entity(8, ellipse(4.0, 0.6)),
        entity(1, nurbs_circle(0.0, 0.0, 3.0)),
    ]
}

#[test]
fn identical_input_gives_identical_files_blobs_and_row_order() {
    let a = export(&sample(), &opts(0.01)).unwrap();
    let b = export(&sample(), &opts(0.01)).unwrap();
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.ledger, b.ledger);
    let (da, db_) = (open(&a.bytes), open(&b.bytes));
    let order = |d: &Db| features(d).iter().map(|r| r["entity_id"].int()).collect::<Vec<_>>();
    assert_eq!(order(&da), vec![5, 2, 9, 8, 1], "rows follow the input order");
    assert_eq!(order(&da), order(&db_));
    let blobs = |d: &Db| features(d).iter().map(|r| r["geom"].blob().to_vec()).collect::<Vec<_>>();
    assert_eq!(blobs(&da), blobs(&db_));
}

#[test]
fn the_file_digest_of_a_straight_line_export_is_pinned() {
    // Straight lines only: no sine, cosine or hypot in the path, so the digest is the same on every host.
    let ents = vec![
        entity(1, polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)])),
        entity(2, polyline(&[(1.0, 1.0), (4.0, 5.0)])),
        entity(3, linear_rational()),
    ];
    let o = opts(0.01).with_last_change("2026-01-01T00:00:00.000Z").unwrap();
    let e = export(&ents, &o).unwrap();
    assert_eq!(e.bytes.len() % 4096, 0);
    assert_eq!(format!("{:016x}", fnv1a(&e.bytes)), PINNED_DIGEST, "{} bytes", e.bytes.len());
}

const PINNED_DIGEST: &str = "237e944c2f4c4db2";

#[test]
fn the_input_order_is_the_row_order_even_when_ids_are_not_sorted() {
    let mut s = sample();
    s.reverse();
    let e = export(&s, &opts(0.01)).unwrap();
    let ids: Vec<i64> = features(&open(&e.bytes)).iter().map(|r| r["entity_id"].int()).collect();
    assert_eq!(ids, vec![1, 8, 9, 2, 5]);
}

// ---- the dependency policy ---------------------------------------------------------------------------

#[test]
fn the_crate_has_no_sqlite_library_no_native_source_and_no_build_machine_discovery() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let manifest = std::fs::read_to_string(format!("{root}/crates/pvec-interchange/Cargo.toml")).unwrap();
    let deps: Vec<&str> = manifest
        .lines()
        .skip_while(|l| !l.starts_with("[dependencies]"))
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .filter_map(|l| l.split('=').next())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    for d in &deps {
        assert!(d.starts_with("pvec-"), "{d}: the only dependencies are the workspace's own crates");
    }
    assert!(!manifest.contains("build ="), "no build script");
    assert!(!std::path::Path::new(&format!("{root}/crates/pvec-interchange/build.rs")).exists());
    let lock = std::fs::read_to_string(format!("{root}/Cargo.lock")).unwrap();
    let names: Vec<&str> =
        lock.lines().filter_map(|l| l.strip_prefix("name = \"")).map(|l| l.trim_end_matches('"')).collect();
    for banned in ["rusqlite", "libsqlite3-sys", "sqlite", "sqlite3-src", "sqlx", "cc", "pkg-config", "vcpkg"]
    {
        assert!(!names.contains(&banned), "Cargo.lock names {banned}");
    }
    assert!(names.iter().all(|n| !n.ends_with("-sys")), "a -sys crate is in Cargo.lock");
}

// ---- the document ---------------------------------------------------------------------------------

#[test]
fn chapter_four_of_the_interchange_document_has_the_decision_table() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/INTERCHANGE.md"))
        .expect("docs/INTERCHANGE.md");
    let at = text.find("## 4. GeoPackage").expect("chapter 4 is the GeoPackage chapter");
    let chapter = &text[at..];
    for word in [
        "CIRCULARSTRING",
        "COMPOUNDCURVE",
        "CURVEPOLYGON",
        "LINESTRING",
        "ExactReparameterized",
        "EquivalentCompound",
        "BoundedApproximation",
        "Unsupported",
        "gpkg_extensions",
        "gpkg_rtree_index",
        "tolerance",
        "measured",
        "tools/gpkg-validate.sh",
    ] {
        assert!(chapter.contains(word), "chapter 4 does not mention {word}");
    }
    for kind in
        ["polyline", "trimmed line", "circle", "ellipse", "bezier", "b-spline", "rational", "composite"]
    {
        assert!(chapter.to_lowercase().contains(kind), "the decision table has no row for {kind}");
    }
}
