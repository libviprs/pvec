//! Advanced geometry codecs (P3.3): Bezier, B-spline and rational B-spline, trimmed splines, and
//! composites that mix them with the basic kinds. The invariants of NP-14, NP-15 and NP-18 are checked
//! on the way in, and nothing here evaluates a spline: the decoded curves are evaluated by
//! `pvec-geometry` like any other.
#![allow(missing_docs)]

mod common;
use common::*;
use pvec_capabilities::ids;
use pvec_codec::*;
use pvec_entity::EntityId;
use pvec_geometry::*;
use pvec_numeric::{MAX_CONTROL_POINTS, MAX_DEGREE};

const DIRECTORY: usize = 24;

fn roundtrip(c: &Curve) -> Curve {
    let enc = encode(&[(id(0), c.clone())]).unwrap();
    decode(&enc.bytes).unwrap().remove(0).1
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_f64(b: &mut [u8], at: usize, v: f64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn clamped_knots(n_cp: usize, degree: usize, interior: &[f64]) -> Vec<f64> {
    assert_eq!(interior.len() + 2 * (degree + 1), n_cp + degree + 1);
    let mut k = vec![0.0; degree + 1];
    k.extend_from_slice(interior);
    k.extend(vec![1.0; degree + 1]);
    k
}

fn cps(n: usize, seed: u64) -> Vec<Point2> {
    let mut r = Rng(seed);
    (0..n).map(|i| pt(i as f64 * 1.37 + r.unit(), r.range(-9.0, 9.0) / 3.0)).collect()
}

/// The spline samples the issue names, with the awkward knot vectors.
fn spline_samples() -> Vec<(&'static str, Curve)> {
    let third = 1.0 / 3.0;
    let mut v: Vec<(&'static str, Curve)> = Vec::new();
    for degree in [1usize, 2, 3, 7, MAX_DEGREE] {
        let c = (0..=degree).map(|i| pt(i as f64 * third, (i * i) as f64 * 0.1 + 0.05)).collect();
        v.push(("bezier", Curve::Bezier(Bezier::new(c).unwrap())));
    }
    // clamped cubic, uneven interior knots
    v.push((
        "bspline-clamped",
        Curve::BSpline(BSpline::new(3, cps(7, 1), clamped_knots(7, 3, &[0.2, 0.55, 0.8])).unwrap()),
    ));
    // full multiplicity interior knot: a C0 break
    v.push((
        "bspline-full-multiplicity",
        Curve::BSpline(
            BSpline::new(3, cps(7, 2), vec![0., 0., 0., 0., 0.5, 0.5, 0.5, 1., 1., 1., 1.]).unwrap(),
        ),
    ));
    // degree 1 degenerate: two control points
    v.push(("bspline-degree-1", Curve::BSpline(BSpline::new(1, cps(2, 3), vec![0., 0., 1., 1.]).unwrap())));
    // unclamped, periodic style: wrapped control points, uniform knots, declared closed
    let ring = vec![pt(0., 0.), pt(2., 0.), pt(2., 2.), pt(0., 2.), pt(0., 0.), pt(2., 0.), pt(2., 2.)];
    let knots: Vec<f64> = (0..10).map(|i| i as f64 * 0.25).collect();
    let periodic = BSpline::new(2, ring, knots).unwrap();
    v.push(("bspline-periodic-style", Curve::BSpline(periodic)));
    // rational, weights differ
    v.push((
        "rational",
        Curve::RationalBSpline(
            RationalBSpline::new(
                2,
                cps(5, 4),
                clamped_knots(5, 2, &[0.4, 0.7]),
                vec![1.0, 0.5, 2.0, 0.75, 1.0],
            )
            .unwrap(),
        ),
    ));
    // rational whose weights are all equal: must stay rational
    v.push((
        "rational-equal-weights",
        Curve::RationalBSpline(
            RationalBSpline::new(2, cps(5, 5), clamped_knots(5, 2, &[0.4, 0.7]), vec![2.5; 5]).unwrap(),
        ),
    ));
    // the nine point rational quadratic circle
    let w = 0.5f64.sqrt();
    let r = 2.0;
    v.push((
        "nurbs-circle",
        Curve::RationalBSpline(
            RationalBSpline::new(
                2,
                vec![
                    pt(r, 0.),
                    pt(r, r),
                    pt(0., r),
                    pt(-r, r),
                    pt(-r, 0.),
                    pt(-r, -r),
                    pt(0., -r),
                    pt(r, -r),
                    pt(r, 0.),
                ],
                vec![0., 0., 0., 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1., 1., 1.],
                vec![1., w, 1., w, 1., w, 1., w, 1.],
            )
            .unwrap(),
        ),
    ));
    // closed Bezier
    v.push((
        "bezier-closed",
        Curve::Bezier(
            Bezier::new(vec![pt(0., 0.), pt(1., 2.), pt(3., 2.), pt(0., 0.)])
                .unwrap()
                .declare_closed()
                .unwrap(),
        ),
    ));
    // trimmed splines, forward and reversed
    let bs = Curve::BSpline(BSpline::new(3, cps(7, 6), clamped_knots(7, 3, &[0.3, 0.6, 0.8])).unwrap());
    v.push(("trimmed-bspline", Curve::Trimmed(Trimmed::new(bs.clone(), 0.1, 0.9, true).unwrap())));
    v.push(("trimmed-bspline-reversed", Curve::Trimmed(Trimmed::new(bs, 0.7, 0.2, false).unwrap())));
    let bz = Curve::Bezier(Bezier::new(cps(4, 7)).unwrap());
    v.push(("trimmed-bezier", Curve::Trimmed(Trimmed::new(bz, 0.25, 0.75, true).unwrap())));
    let rt = Curve::RationalBSpline(
        RationalBSpline::new(2, cps(4, 8), clamped_knots(4, 2, &[0.5]), vec![1.0, 3.0, 0.5, 1.0]).unwrap(),
    );
    v.push(("trimmed-rational", Curve::Trimmed(Trimmed::new(rt, 0.2, 0.8, true).unwrap())));
    v
}

#[test]
fn knots_weights_and_control_points_round_trip_bit_for_bit() {
    for (name, c) in spline_samples() {
        assert_bit_equal(name, &c, &roundtrip(&c));
    }
}

#[test]
fn the_spline_kind_is_preserved_and_equal_weights_stay_rational() {
    for (name, c) in spline_samples() {
        let back = roundtrip(&c);
        assert_eq!(back.kind(), c.kind(), "{name}");
    }
    let (_, eq) = spline_samples().into_iter().find(|(n, _)| *n == "rational-equal-weights").unwrap();
    let back = roundtrip(&eq);
    let Curve::RationalBSpline(r) = &back else { panic!("silently down-converted to {:?}", back.kind()) };
    assert_eq!(r.weights(), &[2.5; 5]);
}

#[test]
fn closed_declarations_on_splines_round_trip() {
    let (_, c) = spline_samples().into_iter().find(|(n, _)| *n == "bezier-closed").unwrap();
    let Curve::Bezier(b) = roundtrip(&c) else { panic!() };
    assert!(b.is_declared_closed());
    let (_, o) = spline_samples().into_iter().find(|(n, _)| *n == "bezier").unwrap();
    let Curve::Bezier(b) = roundtrip(&o) else { panic!() };
    assert!(!b.is_declared_closed());
}

#[test]
fn decoded_splines_evaluate_identically_to_the_originals_and_the_worst_deviation_is_zero() {
    // Geometric exactness on the spline-heavy samples: the decoded curve is the same curve, so the
    // deviation over a dense parameter sweep is exactly zero. The worst case and where it is are
    // reported by the assertion message if that ever stops being true.
    let mut worst = (0.0f64, String::new(), 0.0f64);
    for (name, c) in spline_samples() {
        let back = roundtrip(&c);
        let d = c.domain();
        for i in 0..=1000 {
            let t = d.lo() + d.width() * i as f64 / 1000.0;
            let (a, b) = (c.eval(t).unwrap(), back.eval(t).unwrap());
            let dev = (a.x - b.x).abs().max((a.y - b.y).abs());
            if dev > worst.0 {
                worst = (dev, name.to_string(), t);
            }
        }
    }
    assert_eq!(worst.0, 0.0, "worst deviation {} on {} at t = {}", worst.0, worst.1, worst.2);
}

// ---------------------------------------------------------------- composites with spline segments

#[test]
fn composites_may_hold_splines_and_trimmed_splines_and_keep_their_order() {
    let a = Curve::Bezier(Bezier::new(vec![pt(0., 0.), pt(1., 2.), pt(2., 0.)]).unwrap());
    let b = Curve::BSpline(
        BSpline::new(2, vec![pt(2., 0.), pt(3., 1.), pt(4., 0.)], vec![0., 0., 0., 1., 1., 1.]).unwrap(),
    );
    let rb = RationalBSpline::new(
        2,
        vec![pt(4., 0.), pt(5., 2.), pt(6., 0.)],
        vec![0., 0., 0., 1., 1., 1.],
        vec![1., 2., 1.],
    )
    .unwrap();
    let c = Curve::Trimmed(Trimmed::new(Curve::RationalBSpline(rb), 0.0, 0.5, true).unwrap());
    let end = c.eval(c.domain().hi()).unwrap();
    let d = Curve::Polyline(Polyline::new(vec![end, pt(end.x, end.y + 1.0)]).unwrap());
    let comp = Composite::new(vec![a, b, c, d]).unwrap();
    let curve = Curve::Composite(comp);
    let back = roundtrip(&curve);
    assert_bit_equal("mixed composite", &curve, &back);
    let Curve::Composite(c) = &back else { panic!() };
    let kinds: Vec<CurveKind> = c.segments().iter().map(|s| s.kind()).collect();
    assert_eq!(
        kinds,
        vec![CurveKind::Bezier, CurveKind::BSpline, CurveKind::TrimmedCurve, CurveKind::Polyline]
    );
}

// ---------------------------------------------------------------- capabilities

#[test]
fn spline_blocks_declare_the_registrys_spline_capabilities() {
    let samples = spline_samples();
    let get = |n: &str| samples.iter().find(|(m, _)| *m == n).unwrap().1.clone();
    assert_eq!(encode(&[(id(0), get("bezier"))]).unwrap().capabilities, vec![ids::BEZIER_CURVE]);
    assert_eq!(encode(&[(id(0), get("bspline-clamped"))]).unwrap().capabilities, vec![ids::B_SPLINE_CURVE]);
    assert_eq!(encode(&[(id(0), get("rational"))]).unwrap().capabilities, vec![ids::RATIONAL_B_SPLINE_CURVE]);
    assert_eq!(
        encode(&[(id(0), get("trimmed-bspline"))]).unwrap().capabilities,
        vec![ids::TRIMMED_CURVE, ids::B_SPLINE_CURVE]
    );
}

#[test]
fn every_curve_kind_now_has_a_codec_and_the_section_table_lists_the_spline_sections() {
    let all: Vec<(EntityId, Curve)> = basic_samples()
        .into_iter()
        .chain(spline_samples())
        .enumerate()
        .map(|(i, (_, c))| (id(i as u32), c))
        .collect();
    let enc = encode(&all).unwrap();
    let kinds: Vec<Section> = describe(&enc.bytes).unwrap().into_iter().map(|(s, _)| s).collect();
    assert_eq!(kinds, Section::ALL.to_vec(), "every section kind is used by the combined samples");
    let back = decode(&enc.bytes).unwrap();
    assert_eq!(back.len(), all.len());
}

// ---------------------------------------------------------------- invariants on the way in

/// The offset of the arena of a block with exactly one spline entity: ids (8), offsets (8), arena.
const ARENA: usize = DIRECTORY + 8 + 8 + 8;

fn single_spline(name: &str) -> Vec<u8> {
    let (_, c) = spline_samples().into_iter().find(|(n, _)| *n == name).unwrap();
    encode(&[(id(0), c)]).unwrap().bytes
}

fn refused_rule(bytes: &[u8]) -> &'static str {
    decode(bytes).expect_err("must be refused").rule()
}

// Record header, from docs/GEOMETRY_ENCODING.md: degree, control point count, knot count, flags.
const DEGREE: usize = 0;
const NCP: usize = 4;
const NKNOTS: usize = 8;

#[test]
fn a_knot_vector_whose_length_disagrees_with_degree_and_points_is_a_typed_np14_error() {
    // cubic with 7 control points has 11 knots; claiming degree 2 makes the required length 10
    let mut b = single_spline("bspline-clamped");
    assert_eq!(u32::from_le_bytes(b[ARENA + DEGREE..ARENA + DEGREE + 4].try_into().unwrap()), 3);
    put_u32(&mut b, ARENA + DEGREE, 2);
    assert_eq!(refused_rule(&b), "NP-14");
    // and the other way
    let mut b = single_spline("bspline-clamped");
    put_u32(&mut b, ARENA + DEGREE, 4);
    assert_eq!(refused_rule(&b), "NP-14");
}

#[test]
fn non_monotone_knots_and_too_high_multiplicity_are_refused() {
    let b0 = single_spline("bspline-clamped");
    let knots_at = ARENA + 16 + 7 * 16;
    // swap knots[4] and knots[5]: 0.2 and 0.55 become decreasing
    let mut b = b0.clone();
    let (k4, k5) = (knots_at + 4 * 8, knots_at + 5 * 8);
    let (x, y) = (b[k4..k4 + 8].to_vec(), b[k5..k5 + 8].to_vec());
    b[k4..k4 + 8].copy_from_slice(&y);
    b[k5..k5 + 8].copy_from_slice(&x);
    assert_eq!(refused_rule(&b), "NP-14");
    // an interior knot repeated degree + 1 times (needs eight points to have four interior knots)
    let eight = BSpline::new(3, cps(8, 9), clamped_knots(8, 3, &[0.3, 0.5, 0.6, 0.7])).unwrap();
    let mut b = encode(&[(id(0), Curve::BSpline(eight))]).unwrap().bytes;
    let knots_at = ARENA + 16 + 8 * 16;
    for k in [4, 6, 7] {
        put_f64(&mut b, knots_at + k * 8, 0.5);
    }
    assert_eq!(refused_rule(&b), "NP-14");
    // NaN and infinity in the knots
    let mut b = b0.clone();
    put_f64(&mut b, knots_at + 4 * 8, f64::NAN);
    assert_eq!(refused_rule(&b), "NP-01");
}

#[test]
fn zero_negative_non_finite_and_tiny_weights_are_refused_not_evaluated() {
    let b0 = single_spline("rational");
    let n = u32::from_le_bytes(b0[ARENA + NKNOTS..ARENA + NKNOTS + 4].try_into().unwrap());
    assert_eq!(n, 8, "five points and degree two make eight knots");
    // header, five points of two f64, eight knots, then the five weights
    let weights_at = ARENA + 16 + 5 * 16 + 8 * 8;
    for (bad, rule) in [
        (0.0, "NP-15"),
        (-1.0, "NP-15"),
        (f64::NAN, "NP-01"),
        (f64::INFINITY, "NP-01"),
        (f64::MIN_POSITIVE * 4.0, "NP-15"),
    ] {
        let mut b = b0.clone();
        put_f64(&mut b, weights_at + 8, bad);
        assert_eq!(refused_rule(&b), rule, "weight {bad}");
    }
}

#[test]
fn counts_above_the_maxima_are_refused_before_anything_is_allocated_from_them() {
    let b0 = single_spline("bspline-clamped");
    for ncp in [MAX_CONTROL_POINTS as u32 + 1, u32::MAX, 1 << 28] {
        let mut b = b0.clone();
        put_u32(&mut b, ARENA + NCP, ncp);
        let e = decode(&b).unwrap_err();
        assert!(
            matches!(e, DecodeError::BadRecord | DecodeError::Truncated | DecodeError::TooMany),
            "{ncp}: {e:?}"
        );
    }
    let mut b = b0.clone();
    put_u32(&mut b, ARENA + NKNOTS, u32::MAX);
    assert!(decode(&b).is_err());
    // degree above the maximum
    let mut b = b0.clone();
    put_u32(&mut b, ARENA + DEGREE, MAX_DEGREE as u32 + 1);
    assert!(matches!(refused_rule(&b), "NP-18" | "NP-14"));
}

#[test]
fn a_control_point_hull_that_collapses_to_a_point_is_refused() {
    let b0 = single_spline("bspline-clamped");
    let mut b = b0.clone();
    for i in 0..7 {
        put_f64(&mut b, ARENA + 16 + i * 16, 1.0);
        put_f64(&mut b, ARENA + 16 + i * 16 + 8, 1.0);
    }
    assert_eq!(refused_rule(&b), "NP-08");
}

#[test]
fn a_bezier_record_must_say_a_degree_that_matches_its_points() {
    let b0 = single_spline("bezier");
    // degree-1 bezier is the first sample: two points. Claim three.
    let mut b = b0.clone();
    put_u32(&mut b, ARENA + NCP, 3);
    assert!(decode(&b).is_err());
}

#[test]
fn every_truncation_and_random_corruption_of_a_spline_block_is_refused_or_valid_and_never_panics() {
    let all: Vec<(EntityId, Curve)> =
        spline_samples().into_iter().enumerate().map(|(i, (_, c))| (id(i as u32), c)).collect();
    let good = encode(&all).unwrap().bytes;
    for n in 0..good.len() {
        assert!(decode(&good[..n]).is_err(), "cut at {n}");
    }
    let mut r = Rng(0x5B11);
    for _ in 0..3000 {
        let mut b = good.clone();
        for _ in 0..1 + r.below(4) {
            let at = r.below(b.len());
            b[at] = r.next_u64() as u8;
        }
        if let Ok(v) = decode(&b) {
            // whatever decodes is valid geometry and evaluates without a non-finite result
            for (_, c) in v {
                let d = c.domain();
                for i in 0..=8 {
                    let t = d.lo() + d.width() * i as f64 / 8.0;
                    if let Ok(p) = c.eval(t) {
                        assert!(p.x.is_finite() && p.y.is_finite());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- properties

#[test]
fn random_splines_round_trip_bit_for_bit() {
    let mut r = Rng(0x5911);
    for case in 0..300 {
        let degree = 1 + r.below(6);
        let interior = r.below(6);
        let n = degree + 1 + interior;
        let pts: Vec<Point2> = (0..n).map(|_| pt(r.range(-50.0, 50.0), r.range(-50.0, 50.0))).collect();
        let mut inner: Vec<f64> = (0..interior).map(|_| r.range(0.05, 0.95)).collect();
        inner.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let knots = clamped_knots(n, degree, &inner);
        let curve = if case % 2 == 0 {
            BSpline::new(degree, pts, knots).map(Curve::BSpline)
        } else {
            let w = (0..n).map(|_| r.range(0.5, 3.0)).collect();
            RationalBSpline::new(degree, pts, knots, w).map(Curve::RationalBSpline)
        };
        let Ok(curve) = curve else { continue };
        let curve = if r.below(3) == 0 {
            Trimmed::new(curve, 0.1, 0.8, true).map(Curve::Trimmed).unwrap()
        } else {
            curve
        };
        assert_bit_equal(&format!("case {case}"), &curve, &roundtrip(&curve));
    }
}

// ---------------------------------------------------------------- records crafted byte by byte

/// A block with one entity of a spline section whose record is built here, independently of the
/// encoder, so it can hold numbers the encoder would never write.
fn raw_block(
    section: Section,
    degree: u32,
    cps: &[(f64, f64)],
    knots: &[f64],
    weights: &[f64],
    flags: u32,
) -> Vec<u8> {
    let mut rec = Vec::new();
    for w in [degree, cps.len() as u32, knots.len() as u32, flags] {
        rec.extend_from_slice(&w.to_le_bytes());
    }
    for (x, y) in cps {
        rec.extend_from_slice(&x.to_le_bytes());
        rec.extend_from_slice(&y.to_le_bytes());
    }
    for k in knots.iter().chain(weights) {
        rec.extend_from_slice(&k.to_le_bytes());
    }
    let mut b = Vec::new();
    b.extend_from_slice(b"PVG1");
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes()); // total length, patched below
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&[0; 6]);
    b.extend_from_slice(&section.id().to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes()); // entity id 0
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(rec.len() as u32).to_le_bytes());
    b.extend_from_slice(&rec);
    let n = b.len() as u32;
    b[8..12].copy_from_slice(&n.to_le_bytes());
    b
}

fn ramp(n: usize) -> Vec<(f64, f64)> {
    (0..n).map(|i| (i as f64, ((i * 7) % 5) as f64)).collect()
}

#[test]
fn the_raw_block_builder_agrees_with_the_encoder() {
    // If the independent builder and the encoder ever disagree, the tests below prove nothing.
    let pts = ramp(4);
    let knots = [0., 0., 0., 0., 1., 1., 1., 1.];
    let raw = raw_block(Section::BSpline, 3, &pts, &knots, &[], 0);
    let s = BSpline::new(3, pts.iter().map(|p| pt(p.0, p.1)).collect(), knots.to_vec()).unwrap();
    assert_eq!(raw, encode(&[(id(0), Curve::BSpline(s))]).unwrap().bytes);
}

#[test]
fn counts_just_above_the_maxima_in_a_record_that_really_holds_them_are_refused_as_bad_records() {
    // 65537 control points with matching knots (degree 1): the arrays are all present, so only the
    // cap on the count can refuse this one before the constructors see it.
    let n = MAX_CONTROL_POINTS + 1;
    let knots: Vec<f64> =
        std::iter::once(0.0).chain((0..n).map(|i| i as f64 / (n - 1) as f64)).chain([1.0]).collect();
    let b = raw_block(Section::BSpline, 1, &ramp(n), &knots, &[], 0);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadRecord);
    // and one knot more than the knot maximum, with a small curve
    let many = vec![0.5; pvec_numeric::MAX_KNOTS + 1];
    let b = raw_block(Section::BSpline, 3, &ramp(7), &many, &[], 0);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadRecord);
    // a Bezier curve past degree 16 (18 points), arrays present
    let b = raw_block(Section::Bezier, 17, &ramp(MAX_DEGREE + 2), &[], &[], 0);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadRecord);
    // the largest legal Bezier and B-spline do decode
    assert!(decode(&raw_block(Section::Bezier, 16, &ramp(MAX_DEGREE + 1), &[], &[], 0)).is_ok());
}

#[test]
fn undefined_record_flags_and_inconsistent_bezier_records_are_refused() {
    let knots = [0., 0., 0., 0., 1., 1., 1., 1.];
    let b = raw_block(Section::BSpline, 3, &ramp(4), &knots, &[], 2);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadFlags);
    // a Bezier record states degree = points - 1 and has no knots
    let b = raw_block(Section::Bezier, 5, &ramp(4), &[], &[], 0);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadRecord);
    let b = raw_block(Section::Bezier, 3, &ramp(4), &[0.0], &[], 0);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadRecord);
    assert!(decode(&raw_block(Section::Bezier, 3, &ramp(4), &[], &[], 0)).is_ok());
}

#[test]
fn a_record_must_be_consumed_exactly_and_be_at_least_as_big_as_the_smallest_spline() {
    let good = raw_block(Section::Bezier, 3, &ramp(4), &[], &[], 0);
    let offs_at = DIRECTORY + 8 + 8; // directory, ids (padded), then the offset table
    // eight spare bytes at the end of the record, covered by the offsets and the total length
    let mut b = good.clone();
    b.extend_from_slice(&[0; 8]);
    let (n, len) = (b.len() as u32, u32::from_le_bytes(b[offs_at + 4..offs_at + 8].try_into().unwrap()) + 8);
    b[8..12].copy_from_slice(&n.to_le_bytes());
    b[offs_at + 4..offs_at + 8].copy_from_slice(&len.to_le_bytes());
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadRecord);
    // an offset table that claims a record smaller than any spline can be
    let mut b = good.clone();
    b[offs_at + 4..offs_at + 8].copy_from_slice(&40u32.to_le_bytes());
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadOffsets);
}

#[test]
fn trimmed_spline_and_composite_spline_segment_words_are_checked() {
    // trimmed spline record: basis tag, reversed word, start, span, then the spline record
    let (_, c) = spline_samples().into_iter().find(|(n, _)| *n == "trimmed-bspline").unwrap();
    let b0 = encode(&[(id(0), c)]).unwrap().bytes;
    assert_eq!(&b0[ARENA..ARENA + 4], &2u32.to_le_bytes(), "B-spline basis tag");
    let mut b = b0.clone();
    put_u32(&mut b, ARENA + 4, 2);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadFlags);
    let mut b = b0.clone();
    put_u32(&mut b, ARENA, 9);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::BadTag(9));

    // a spline segment of a composite: the word after the tag is reserved
    let a = Curve::Bezier(Bezier::new(vec![pt(0., 0.), pt(1., 2.), pt(2., 0.)]).unwrap());
    let b0 = encode(&[(id(0), Curve::Composite(Composite::new(vec![a]).unwrap()))]).unwrap().bytes;
    let seg = DIRECTORY + 8 + 8 + 8 + 8 + 8; // ids, flags, offsets, then the record header (count and zero)
    assert_eq!(&b0[seg..seg + 4], &5u32.to_le_bytes(), "Bezier segment tag");
    let mut b = b0.clone();
    put_u32(&mut b, seg + 4, 1);
    assert_eq!(decode(&b).unwrap_err(), DecodeError::ReservedNotZero);
}
