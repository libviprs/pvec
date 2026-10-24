//! Writes the sample GeoPackages `tools/gpkg-validate.sh` checks with the GeoPackage validator and an
//! independent GIS reader: `cargo run -p pvec-interchange --example gpkg_samples -- <directory>`.
//! The directory has to exist. Nothing here is read back; the script does that, in a container.

use pvec_entity::EntityId;
use pvec_geometry::*;
use pvec_interchange::gpkg::*;
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2};

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn ent(n: u32, curve: Curve) -> GpkgEntity {
    GpkgEntity { entity_id: EntityId::new(n).unwrap(), curve }
}

fn arc(cx: f64, cy: f64, r: f64, t1: f64, t2: f64) -> Curve {
    let c = Circle::new(Placement2::new(p(cx, cy), Vector2::new(1.0, 0.0)).unwrap(), r).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Circle(c), t1, t2, true).unwrap())
}

fn segment(a: Point2, b: Point2) -> Curve {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    let line = Line::new(a, Vector2::new(dx / len, dy / len)).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, len, true).unwrap())
}

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

fn mixed() -> Vec<GpkgEntity> {
    let comp = Composite::new(vec![
        segment(p(0.0, 20.0), p(4.0, 20.0)),
        arc(4.0, 23.0, 3.0, -FRAC_PI_2, 0.0),
        Curve::Polyline(Polyline::new(vec![p(7.0, 23.0), p(7.0, 28.0), p(2.0, 28.0)]).unwrap()),
    ])
    .unwrap();
    vec![
        ent(1, Curve::Polyline(Polyline::new(vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0)]).unwrap())),
        ent(2, segment(p(0.0, 5.0), p(6.0, 8.0))),
        ent(3, arc(10.0, 0.0, 5.0, 0.2, 2.4)),
        ent(4, arc(20.0, 0.0, 5.0, 0.5, 5.5)),
        ent(
            5,
            Curve::Circle(
                Circle::new(Placement2::new(p(30.0, 0.0), Vector2::new(1.0, 0.0)).unwrap(), 4.0).unwrap(),
            ),
        ),
        ent(6, Curve::Ellipse(Ellipse::from_major_axis(p(40.0, 0.0), Vector2::new(6.0, 0.0), 0.4).unwrap())),
        ent(
            7,
            Curve::Bezier(Bezier::new(vec![p(0.0, 10.0), p(2.0, 15.0), p(7.0, 15.0), p(9.0, 10.0)]).unwrap()),
        ),
        ent(
            8,
            Curve::BSpline(
                BSpline::new(
                    3,
                    vec![
                        p(10.0, 10.0),
                        p(11.0, 13.0),
                        p(13.0, 14.0),
                        p(15.0, 11.0),
                        p(17.0, 13.0),
                        p(19.0, 10.0),
                    ],
                    vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0],
                )
                .unwrap(),
            ),
        ),
        ent(
            9,
            Curve::RationalBSpline(
                RationalBSpline::new(
                    2,
                    vec![p(20.0, 10.0), p(25.0, 18.0), p(30.0, 10.0)],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    vec![1.0, 3.0, 1.0],
                )
                .unwrap(),
            ),
        ),
        ent(10, nurbs_circle(40.0, 12.0, 4.0)),
        ent(11, Curve::Composite(comp)),
        ent(12, Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap())),
    ]
}

fn many(n: u32) -> Vec<GpkgEntity> {
    (1..=n)
        .map(|i| {
            let (x, y) = (f64::from(i % 97) * 3.0, f64::from(i / 97) * 3.0);
            if i % 5 == 0 {
                ent(i, arc(x, y, 1.0, 0.0, 2.0 + f64::from(i % 3)))
            } else {
                ent(i, Curve::Polyline(Polyline::new(vec![p(x, y), p(x + 1.5, y + 0.5)]).unwrap()))
            }
        })
        .collect()
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: gpkg_samples <existing directory>");
    let opts = GpkgOptions::new(0.001).unwrap().with_last_change("2026-10-24T00:00:00.000Z").unwrap();
    let sets: [(&str, Vec<GpkgEntity>); 4] = [
        ("empty", Vec::new()),
        ("lines", mixed().into_iter().take(2).collect()),
        ("mixed", mixed()),
        ("many", many(1500)),
    ];
    for (name, entities) in sets {
        let e = export(&entities, &opts).expect("export");
        std::fs::write(format!("{dir}/{name}.gpkg"), &e.bytes).expect("write");
        std::fs::write(format!("{dir}/{name}.ledger"), e.ledger.to_text()).expect("write");
        println!("{name}: {} entities, {} bytes", entities.len(), e.bytes.len());
    }
}
