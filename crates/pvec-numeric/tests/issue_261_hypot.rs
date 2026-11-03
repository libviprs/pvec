//! Issue 261: `f64::hypot` goes to the C library of the machine and its last bit differs between
//! x86_64 and arm64 glibc, so `detmath::hypot` builds it from `+ - * /` and `sqrt` alone.

use pvec_numeric::detmath::hypot;

fn ulps(a: f64, b: f64) -> u64 {
    a.to_bits().abs_diff(b.to_bits())
}

#[test]
fn exact_cases_stay_exact() {
    // Guaranteed, not lucky: the smaller over the larger is 3/4 for each of these, which is a double,
    // and 1 + 9/16 = 25/16 and its square root 5/4 and the final product by 4 are all exact.
    assert_eq!(hypot(3.0, 4.0), 5.0);
    assert_eq!(hypot(-3.0, 4.0), 5.0);
    assert_eq!(hypot(4.0, -3.0), 5.0);
    // 5 and 12 give r = 5/12, which is not a double, so 13 is within an ulp and nothing more is promised.
    assert!(ulps(hypot(5.0, 12.0), 13.0) <= 1);
    let (up, down) = (2.0f64.powi(500), 2.0f64.powi(-500));
    assert_eq!(hypot(3.0 * up, 4.0 * up), 5.0 * up);
    assert_eq!(hypot(3.0 * down, 4.0 * down), 5.0 * down);
}

#[test]
fn a_zero_component_gives_the_other_magnitude() {
    assert_eq!(hypot(0.0, 0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(hypot(-0.0, -0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(hypot(0.0, -7.5), 7.5);
    assert_eq!(hypot(-7.5, 0.0), 7.5);
    assert_eq!(hypot(f64::MIN_POSITIVE / 4.0, 0.0), f64::MIN_POSITIVE / 4.0);
}

#[test]
fn infinity_and_nan_follow_ieee() {
    assert_eq!(hypot(f64::INFINITY, 1.0), f64::INFINITY);
    assert_eq!(hypot(-1.0, f64::NEG_INFINITY), f64::INFINITY);
    assert_eq!(hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
    assert_eq!(hypot(f64::NEG_INFINITY, f64::NAN), f64::INFINITY);
    assert!(hypot(f64::NAN, 1.0).is_nan());
    assert!(hypot(2.0, f64::NAN).is_nan());
}

#[test]
fn neither_overflow_nor_underflow_in_the_squares() {
    let (up, down) = (2.0f64.powi(700), 2.0f64.powi(-700));
    let big = hypot(up, up);
    assert!(big.is_finite());
    assert_eq!(big.to_bits(), (up * std::f64::consts::SQRT_2).to_bits());
    let small = hypot(down, down);
    assert!(small > 0.0);
    assert_eq!(small.to_bits(), (down * std::f64::consts::SQRT_2).to_bits());
    // the true length is above MAX, so infinity is the right answer
    assert_eq!(hypot(f64::MAX, f64::MAX), f64::INFINITY);
    assert!(hypot(f64::MAX, 1.0).is_finite());
    // a subnormal pair
    let tiny = f64::from_bits(3);
    assert_eq!(hypot(tiny, tiny).to_bits(), f64::from_bits(4).to_bits());
}

#[test]
fn symmetric_in_sign_and_order() {
    for i in 1..200 {
        let (x, y) = (f64::from(i) * 0.731, f64::from(i * 7 % 101) * -0.419);
        let h = hypot(x, y).to_bits();
        assert_eq!(hypot(y, x).to_bits(), h);
        assert_eq!(hypot(-x, y).to_bits(), h);
        assert_eq!(hypot(x, -y).to_bits(), h);
    }
}

#[test]
fn within_two_ulps_of_the_c_library() {
    let mut s = 0x2545_f491_4f6c_dd1d_u64;
    for _ in 0..20_000 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let x = (s >> 11) as f64 / (1u64 << 53) as f64 * 2000.0 - 1000.0;
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let y = (s >> 11) as f64 / (1u64 << 53) as f64 * 2000.0 - 1000.0;
        assert!(ulps(hypot(x, y), x.hypot(y)) <= 2, "hypot({x}, {y})");
    }
}

#[test]
fn the_arithmetic_is_pinned_bit_for_bit_so_every_machine_gets_the_same_length() {
    assert_eq!(hypot(0.28, 0.96).to_bits(), HYPOT_0_28_0_96);
    assert_eq!(hypot(1.0, 1.0).to_bits(), std::f64::consts::SQRT_2.to_bits());
    assert_eq!(hypot(0.1, 0.2).to_bits(), HYPOT_0_1_0_2);
    assert_eq!(hypot(123.456, -789.012).to_bits(), HYPOT_123_456_789_012);
}

const HYPOT_0_28_0_96: u64 = 0x3ff0_0000_0000_0000;
const HYPOT_0_1_0_2: u64 = 0x3fcc_9f25_c5bf_edda;
const HYPOT_123_456_789_012: u64 = 0x4088_f4e5_9fc8_ba4f;

#[test]
fn normalise_direction_and_is_unit_use_it() {
    use pvec_numeric::Policy;
    for (dx, dy) in [(0.1, 0.2), (123.456, -789.012), (-0.57735, 0.8165), (33000.0, 44000.0)] {
        let m = f64::max(f64::abs(dx), f64::abs(dy));
        let (sx, sy) = (dx / m, dy / m);
        let n = hypot(sx, sy);
        let d = Policy::V1.normalise_direction(dx, dy).unwrap();
        assert_eq!(d[0].to_bits(), (sx / n).to_bits(), "({dx}, {dy})");
        assert_eq!(d[1].to_bits(), (sy / n).to_bits(), "({dx}, {dy})");
        assert!(Policy::V1.is_unit(d));
    }
}

// Inputs found by search where the C library of x86_64 glibc and `detmath::hypot` differ by one ulp
// on opposite sides of a tolerance, so the decision below changes if a predicate goes back to
// `f64::hypot`. Each holds the bits, so no literal can be rounded differently.
const SEG: [f64; 2] = [f64::from_bits(0x3d65_ac3a_dd10_d876), f64::from_bits(0x3d6b_b78b_f4a3_9cf4)];

#[test]
fn a_hull_whose_far_point_is_exactly_the_floor_away_is_a_point() {
    use pvec_numeric::{HullClass, Policy};
    // detmath::hypot(SEG) is the NP-06 floor itself (degenerate); the C library says one ulp more.
    assert_eq!(Policy::V1.hull_class(&[[0.0, 0.0], SEG]), HullClass::Point);
}

#[test]
fn the_collinear_band_uses_the_deterministic_length() {
    use pvec_numeric::{Orientation, Policy};
    let b = [f64::from_bits(0x3d79_d1d5_4ab5_98a5), f64::from_bits(0x3d67_386a_46d6_b215)];
    let c = [f64::from_bits(0x3d76_38e7_30e1_d748), f64::from_bits(0xbd62_97ba_c57f_b3aa)];
    // detmath::hypot puts this determinant inside the band; the C library's longer edge by one ulp
    // moves the band just under it.
    assert_eq!(Policy::V1.orient2d([0.0, 0.0], b, c), Orientation::Collinear);
}
