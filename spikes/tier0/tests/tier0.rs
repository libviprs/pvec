//! P1.7 acceptance checks. Each one is built so it fails if the property it names stops holding.

use pvec_fixtures::{Entity, Geom, generate};
use tier0_spike::model::reference_curve;
use tier0_spike::preview::*;
use tier0_spike::tier::*;

const SCREEN_PX: f64 = 1920.0;
const BUDGET_PX: f64 = 0.5;

/// The first `n` entities of a fixture, so the slow verification stays inside the gate's time.
fn head(mut d: pvec_fixtures::Drawing, n: usize) -> pvec_fixtures::Drawing {
    d.entities.truncate(n);
    d
}

fn tol_for(extent_w: f64) -> f64 {
    BUDGET_PX / (SCREEN_PX / extent_w)
}

#[test]
fn every_entity_is_exact_previewed_or_listed_with_a_reason() {
    for d in [generate::small_synthetic(), head(generate::spline_heavy(), 400)] {
        let t = assign(&d, &Policy::DEFAULT, tol_for(d.extent[2]));
        assert_eq!(t.exact.len() + t.previews.len() + t.neither.len(), d.entities.len(), "{}", d.name);
        check_identity(&d, &t).unwrap();
    }
}

#[test]
fn a_preview_replaces_exactly_the_entity_it_names_and_the_identity_survives_reassignment() {
    let d = generate::small_synthetic();
    let tol = tol_for(d.extent[2]);
    let a = assign(&d, &Policy::DEFAULT, tol);
    let b = assign(&d, &Policy { exact_ellipses: true, ..Policy::DEFAULT }, tol);
    // Under the second policy, every elliptic preview of the first becomes an exact record with the
    // same id: the identity is the entity's, not the tier's.
    let ellipse_previews: Vec<u32> =
        a.previews.iter().map(|p| p.replaces).filter(|&id| d.entities[id as usize].geom.tag() == 3).collect();
    assert!(!ellipse_previews.is_empty());
    for id in &ellipse_previews {
        assert!(b.exact.contains(id) && !b.previews.iter().any(|p| p.replaces == *id));
    }
    // A preview that survives both policies carries the same replacement id and resolves to the
    // same exact entity.
    for p in &a.previews {
        if let Some(q) = b.previews.iter().find(|q| q.replaces == p.replaces) {
            assert_eq!(d.entities[p.replaces as usize].id, q.replaces);
        }
    }
}

#[test]
fn a_preview_pointing_at_the_wrong_entity_is_caught() {
    let d = generate::small_synthetic();
    let mut t = assign(&d, &Policy::DEFAULT, tol_for(d.extent[2]));
    // Forge: two previews claim one id, and one claims an id that does not exist.
    let dup = t.previews[0].clone();
    t.previews.push(dup);
    assert!(check_identity(&d, &t).is_err());
    t.previews.pop();
    t.previews[0].replaces = u32::MAX;
    assert!(check_identity(&d, &t).is_err());
}

#[test]
fn chosen_previews_stay_within_the_measured_budget() {
    for d in [generate::small_synthetic(), head(generate::spline_heavy(), 400)] {
        let tol = tol_for(d.extent[2]);
        let t = assign(&d, &Policy::DEFAULT, tol);
        for p in t.previews.iter().step_by(7) {
            check_bound(&d.entities[p.replaces as usize], p, tol).unwrap();
        }
        assert!(t.chosen_dev_max <= tol, "{}: {} > {tol}", d.name, t.chosen_dev_max);
    }
}

#[test]
fn a_coarse_preview_fails_the_bound_check() {
    // One chord for a whole circle is a degenerate preview. The bound check must reject it.
    let e = Entity {
        id: 0,
        geom: Geom::Ellipse {
            c: [0.0, 0.0],
            a: 100.0,
            b: 60.0,
            rot: 0.3,
            t0: 0.0,
            t1: core::f64::consts::TAU,
        },
    };
    let coarse = quantise(0, Form::SagittaPolyline, false, &[[100.0, 0.0], [-100.0, 0.0]]);
    assert!(check_bound(&e, &coarse, 1.0).is_err());
    // And a stub of a large entity fails a tight budget but would pass a loose one.
    let stub = quantise(
        0,
        Form::BboxStub,
        true,
        &[[-100.0, -100.0], [100.0, -100.0], [100.0, 100.0], [-100.0, 100.0]],
    );
    assert!(check_bound(&e, &stub, 1.0).is_err());
    assert!(check_bound(&e, &stub, 1000.0).is_ok());
}

#[test]
fn the_sagitta_polyline_meets_a_rigorous_bound_on_a_circle() {
    // Sagitta of a chord spanning angle th on radius r is r (1 - cos(th/2)); the polyline's
    // measured deviation must not exceed the budget it was built for.
    let g = Geom::Circle { c: [10.0, -5.0], r: 500.0 };
    let c = reference_curve(&g).unwrap();
    let tol = 2.0;
    let pl = sagitta_polyline(&c, tol, 8).unwrap();
    let dense = dense_samples(&c, 2048);
    assert!(deviation(&dense, &pl, false) <= tol);
    let worst_chord = pl.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).fold(0.0, f64::max);
    let th = 2.0 * (worst_chord / (2.0 * 500.0)).asin();
    assert!(500.0 * (1.0 - (th / 2.0).cos()) <= tol);
}

#[test]
fn invalid_geometry_lands_in_neither_with_the_evaluator_reason() {
    let mut d = generate::small_synthetic();
    d.entities[0].geom = Geom::Line { a: [1.0, 1.0], b: [1.0, 1.0] };
    d.entities[1].geom = Geom::Circle { c: [0.0, 0.0], r: 0.0 };
    let t = assign(&d, &Policy::DEFAULT, 1.0);
    assert!(t.neither.iter().any(|(id, r)| *id == 0 && matches!(r, Reason::Refused(_))));
    assert!(t.neither.iter().any(|(id, r)| *id == 1 && matches!(r, Reason::Refused(_))));
    check_identity(&d, &t).unwrap();
}

#[test]
fn the_stream_round_trips_and_hostile_streams_are_refused() {
    let d = generate::small_synthetic();
    let t = assign(&d, &Policy::DEFAULT, tol_for(d.extent[2]));
    let bytes = encode(&d, &t);
    let r = decode(&bytes).unwrap();
    assert_eq!((r.exact, r.previews), (t.exact.len(), t.previews.len()));
    let ids: Vec<u32> = t.previews.iter().map(|p| p.replaces).collect();
    assert_eq!(r.preview_ids, ids);
    // Every truncation is an error, never a panic.
    for cut in (0..bytes.len()).step_by(37) {
        assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    // A header that promises more records than there are bytes.
    let mut lie = bytes.clone();
    lie[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&lie).is_err());
    // Bad magic, trailing bytes, an absurd polyline length, an unknown tag.
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(decode(&bad).is_err());
    let mut tail = bytes.clone();
    tail.push(0);
    assert!(decode(&tail).is_err());
    let mut poly = Vec::new();
    poly.extend_from_slice(b"T0P1");
    poly.extend_from_slice(&1u32.to_le_bytes());
    poly.extend_from_slice(&0u32.to_le_bytes());
    poly.extend_from_slice(&0u32.to_le_bytes());
    poly.push(4);
    poly.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&poly).is_err());
    poly[12] = 9;
    assert!(decode(&poly).is_err());
}

#[test]
fn the_write_up_never_calls_a_preview_exact() {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/spikes/01-7-tier0.md"));
    let Ok(doc) = doc else { return }; // the document is written after the first measurement run
    for para in doc.split("\n\n") {
        let l = para.to_lowercase();
        if l.contains("preview") && (l.contains("mathematically exact") || l.contains("geometrically exact"))
        {
            assert!(
                l.contains(" not ") || l.contains("never") || l.contains("no preview"),
                "claims exactness: {para}"
            );
        }
    }
}
