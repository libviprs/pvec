//! Basic geometry codecs (P3.2): bit-exact round trips, kind preservation, trim and composite
//! fidelity, determinism, and the capability ids the registry hands out.
#![allow(missing_docs)]

mod common;
use common::*;
use pvec_capabilities::{Registry, ids};
use pvec_codec::*;
use pvec_entity::EntityId;
use pvec_geometry::*;

fn roundtrip(entities: &[(EntityId, Curve)]) -> Vec<(EntityId, Curve)> {
    let enc = encode(entities).unwrap();
    decode(&enc.bytes).unwrap()
}

#[test]
fn every_basic_sample_round_trips_bit_for_bit() {
    for (name, curve) in basic_samples() {
        let back = roundtrip(&[(id(0), curve.clone())]);
        assert_eq!(back.len(), 1, "{name}");
        assert_eq!(back[0].0, id(0), "{name}");
        assert_bit_equal(name, &curve, &back[0].1);
    }
}

#[test]
fn a_mixed_block_round_trips_with_ids_attached_to_the_right_curves() {
    let samples = basic_samples();
    // ids deliberately not in sample order
    let ents: Vec<(EntityId, Curve)> = samples
        .iter()
        .enumerate()
        .map(|(i, (_, c))| (id((samples.len() - 1 - i) as u32 * 3), c.clone()))
        .collect();
    let back = roundtrip(&ents);
    assert_eq!(back.len(), ents.len());
    for (eid, curve) in &ents {
        let (_, got) = back.iter().find(|(i, _)| i == eid).unwrap_or_else(|| panic!("{eid} is missing"));
        assert_bit_equal(&format!("{eid}"), curve, got);
    }
}

#[test]
fn coordinates_that_do_not_fit_in_f32_survive_so_an_f32_on_the_path_turns_this_red() {
    // 0.1 and 1/3 are not exactly representable in f32, and 2^40 + 1/1024 needs 51 significant bits.
    let big = (1u64 << 39) as f64 + 1.0 / 1024.0;
    let p = Polyline::new(vec![pt(0.1, 1.0 / 3.0), pt(big, -big)]).unwrap();
    let back = roundtrip(&[(id(0), Curve::Polyline(p.clone()))]);
    let Curve::Polyline(q) = &back[0].1 else { panic!("kind changed") };
    for (a, b) in p.vertices().iter().zip(q.vertices()) {
        assert_eq!(a.x.to_bits(), b.x.to_bits());
        assert_eq!(a.y.to_bits(), b.y.to_bits());
        assert_ne!(f64::from(a.x as f32).to_bits(), a.x.to_bits(), "the fixture must not fit in f32");
    }
}

#[test]
fn curve_kind_is_preserved_per_sample() {
    for (name, curve) in basic_samples() {
        let back = roundtrip(&[(id(0), curve.clone())]);
        assert_eq!(back[0].1.kind(), curve.kind(), "{name}");
        // an arc stays a trimmed circle and a polyline stays a polyline, all the way down
        match (&curve, &back[0].1) {
            (Curve::Trimmed(a), Curve::Trimmed(b)) => {
                assert_eq!(a.basis().kind(), b.basis().kind(), "{name}")
            }
            (Curve::Composite(a), Curve::Composite(b)) => {
                let k = |c: &Composite| c.segments().iter().map(|s| s.kind()).collect::<Vec<_>>();
                assert_eq!(k(a), k(b), "{name}");
            }
            _ => {}
        }
    }
}

#[test]
fn an_arc_is_not_emitted_as_a_polyline_and_a_polyline_is_not_promoted() {
    let samples = basic_samples();
    let get = |n: &str| samples.iter().find(|(m, _)| *m == n).unwrap().1.clone();
    let arc = encode(&[(id(0), get("arc"))]).unwrap();
    assert_eq!(describe(&arc.bytes).unwrap(), vec![(Section::TrimmedCircle, 1)]);
    let ell = encode(&[(id(0), get("elliptical-arc"))]).unwrap();
    assert_eq!(describe(&ell.bytes).unwrap(), vec![(Section::TrimmedEllipse, 1)]);
    let seg = encode(&[(id(0), get("segment"))]).unwrap();
    assert_eq!(describe(&seg.bytes).unwrap(), vec![(Section::TrimmedLine, 1)]);
    let poly = encode(&[(id(0), get("polyline-100"))]).unwrap();
    assert_eq!(describe(&poly.bytes).unwrap(), vec![(Section::Polyline, 1)]);
    let circle = encode(&[(id(0), get("circle")), (id(1), get("ellipse")), (id(2), get("line"))]).unwrap();
    assert_eq!(
        describe(&circle.bytes).unwrap(),
        vec![(Section::Line, 1), (Section::Circle, 1), (Section::Ellipse, 1)]
    );
}

#[test]
fn trim_domains_round_trip_including_inverted_and_periodic_cases() {
    let circle = Circle::new(place(0.3, 0.4, 1.0, 2.0), 2.5).unwrap();
    let cases = [
        (0.3, 2.9, true),
        (4.0, 1.0, false),
        (5.5, 1.0, true),
        (-1.0, 1.0, true),
        (1.0, -3.0, false),
        (0.0, core::f64::consts::TAU, true),
        (7.0, 7.0 + 1.0 / 1024.0, true),
        (core::f64::consts::TAU - 1.0 / 4096.0, 1.0, true),
    ];
    for (t1, t2, sense) in cases {
        let t = Trimmed::new(Curve::Circle(circle), t1, t2, sense).unwrap();
        let before = t.trim();
        let back = roundtrip(&[(id(0), Curve::Trimmed(t.clone()))]);
        let Curve::Trimmed(u) = &back[0].1 else { panic!("kind changed") };
        let after = u.trim();
        let why = format!("trim ({t1}, {t2}, {sense})");
        assert_eq!(before.start.to_bits(), after.start.to_bits(), "{why}: start");
        assert_eq!(before.span.to_bits(), after.span.to_bits(), "{why}: span");
        assert_eq!(before.reversed, after.reversed, "{why}: reversed");
        // and the decoded arc still ends where the original ended
        let (d0, d1) = (t.domain(), u.domain());
        assert_eq!((d0.lo().to_bits(), d0.hi().to_bits()), (d1.lo().to_bits(), d1.hi().to_bits()));
    }
}

#[test]
fn a_composites_segment_order_and_closed_flag_round_trip() {
    let sq = square(0);
    assert!(sq.is_declared_closed());
    let back = roundtrip(&[(id(0), Curve::Composite(sq.clone()))]);
    assert_bit_equal("square", &Curve::Composite(sq.clone()), &back[0].1);
    let Curve::Composite(c) = &back[0].1 else { panic!("kind changed") };
    assert!(c.is_declared_closed());

    // An open composite stays open: the flag is not inferred from the geometry.
    let open = Composite::new(sq.segments()[..3].to_vec()).unwrap();
    let back = roundtrip(&[(id(0), Curve::Composite(open))]);
    let Curve::Composite(c) = &back[0].1 else { panic!("kind changed") };
    assert!(!c.is_declared_closed());
    assert_eq!(c.segments().len(), 3);
}

#[test]
fn reordering_composite_segments_changes_the_decoded_curve_and_a_test_detects_it() {
    let (a, b) = (square(0), square(1));
    assert_ne!(Curve::Composite(a.clone()), Curve::Composite(b.clone()), "the fixtures must differ");
    let back_a = roundtrip(&[(id(0), Curve::Composite(a.clone()))]);
    let back_b = roundtrip(&[(id(0), Curve::Composite(b.clone()))]);
    assert_eq!(back_a[0].1, Curve::Composite(a.clone()));
    assert_eq!(back_b[0].1, Curve::Composite(b));
    assert_ne!(back_a[0].1, back_b[0].1);
    // the first segment of each is the one that was first on the way in
    let first = |c: &Curve| match c {
        Curve::Composite(c) => c.segments()[0].eval(0.0).unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(first(&back_a[0].1), pt(0.0, 0.0));
    assert_eq!(first(&back_b[0].1), pt(4.0, 0.0));
}

#[test]
fn encoding_is_deterministic_and_order_within_a_kind_is_input_order() {
    let ents = entities(&basic_samples());
    assert_eq!(encode(&ents).unwrap().bytes, encode(&ents).unwrap().bytes);
    let mut swapped = ents.clone();
    swapped.swap(0, 3); // a line and a segment: different kinds, so grouping hides the swap
    swapped.swap(6, 7); // two arcs: the same kind, so the order is visible
    assert_ne!(encode(&ents).unwrap().bytes, encode(&swapped).unwrap().bytes);
    let back = decode(&encode(&swapped).unwrap().bytes).unwrap();
    let arcs: Vec<EntityId> =
        back.iter().filter(|(_, c)| matches!(c, Curve::Trimmed(_))).map(|(i, _)| *i).collect();
    let pos = |n: u32| arcs.iter().position(|i| i.index() == n).unwrap();
    assert!(pos(7) < pos(6), "within a kind the input order is kept");
}

#[test]
fn the_empty_set_round_trips() {
    let enc = encode(&[]).unwrap();
    assert!(decode(&enc.bytes).unwrap().is_empty());
    assert!(describe(&enc.bytes).unwrap().is_empty());
    assert!(enc.capabilities.is_empty());
}

#[test]
fn duplicate_ids_are_refused_by_the_encoder() {
    let c = basic_samples().remove(0).1;
    let err = encode(&[(id(5), c.clone()), (id(5), c)]).unwrap_err();
    assert_eq!(err, EncodeError::DuplicateId(id(5)));
}

#[test]
fn a_spline_is_refused_not_degraded_until_the_advanced_codecs_exist() {
    let b = Bezier::new(vec![pt(0., 0.), pt(1., 2.), pt(2., 0.)]).unwrap();
    let err = encode(&[(id(0), Curve::Bezier(b))]).unwrap_err();
    assert_eq!(err, EncodeError::Unsupported(CurveKind::Bezier));
}

// ---------------------------------------------------------------- capabilities come from the registry

#[test]
fn capability_ids_are_the_registrys_not_the_codecs() {
    let r = Registry::v1();
    for (kind, want) in [
        (CurveKind::Line, ids::LINE),
        (CurveKind::Circle, ids::CIRCLE),
        (CurveKind::Ellipse, ids::ELLIPSE),
        (CurveKind::Polyline, ids::POLYLINE),
        (CurveKind::TrimmedCurve, ids::TRIMMED_CURVE),
        (CurveKind::CompositeCurve, ids::COMPOSITE_CURVE),
        (CurveKind::Bezier, ids::BEZIER_CURVE),
        (CurveKind::BSpline, ids::B_SPLINE_CURVE),
        (CurveKind::RationalBSpline, ids::RATIONAL_B_SPLINE_CURVE),
    ] {
        assert_eq!(capability_for(kind), Some(want), "{kind:?}");
        assert!(r.get(want).is_some());
    }
}

#[test]
fn a_block_declares_the_capabilities_it_needs_including_the_bases_of_trimmed_curves() {
    let samples = basic_samples();
    let get = |n: &str| samples.iter().find(|(m, _)| *m == n).unwrap().1.clone();
    let arc = encode(&[(id(0), get("arc"))]).unwrap();
    assert_eq!(arc.capabilities, vec![ids::CIRCLE, ids::TRIMMED_CURVE]);
    let seg = encode(&[(id(0), get("segment"))]).unwrap();
    assert_eq!(seg.capabilities, vec![ids::LINE, ids::TRIMMED_CURVE]);
    let comp = encode(&[(id(0), get("composite"))]).unwrap();
    assert_eq!(
        comp.capabilities,
        vec![ids::LINE, ids::POLYLINE, ids::CIRCLE, ids::TRIMMED_CURVE, ids::COMPOSITE_CURVE]
    );
    let mixed = encode(&entities(&samples)).unwrap();
    let mut sorted = mixed.capabilities.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(mixed.capabilities, sorted, "sorted by id, no duplicates");
}

#[test]
fn the_wire_version_and_section_names_are_documented() {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/GEOMETRY_ENCODING.md"))
            .expect("docs/GEOMETRY_ENCODING.md exists");
    let doc = doc.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(doc.contains(&format!("version {FORMAT_VERSION}")));
    for s in Section::ALL {
        assert!(doc.contains(&format!("`{}`", s.name())), "the spec does not describe section {}", s.name());
    }
}
