//! Issue 121: the deterministic arctangent lives in `pvec-numeric`, where the production writer and
//! the model can reach it (it used to be test support only).

use pvec_numeric::detmath::{atan, atan2};
use std::f64::consts::{FRAC_PI_2, PI};

fn ulps(a: f64, b: f64) -> u64 {
    a.to_bits().abs_diff(b.to_bits())
}

#[test]
fn the_arithmetic_is_pinned_bit_for_bit_so_every_machine_gets_the_same_angle() {
    assert_eq!(atan2(0.28, 0.96).to_bits(), 0x3fd2_29ae_c476_38dd_u64);
    assert_eq!(atan(0.4).to_bits(), 0x3fd8_5a37_6b67_7dc0_u64);
}

#[test]
fn atan_and_atan2_are_within_one_ulp_of_the_c_library() {
    for i in -40..=40 {
        let x = f64::from(i) * 0.37;
        assert!(ulps(atan(x), x.atan()) <= 1, "atan({x})");
        for j in -40..=40 {
            let y = f64::from(j) * 0.29;
            if x != 0.0 || y != 0.0 {
                assert!(ulps(atan2(y, x), y.atan2(x)) <= 1, "atan2({y}, {x})");
            }
        }
    }
}

#[test]
fn the_axes_are_exact_and_the_sign_of_zero_is_kept() {
    assert_eq!(atan2(0.0, -1.0), PI);
    assert_eq!(atan2(-0.0, -1.0), -PI);
    assert_eq!(atan2(1.0, 0.0), FRAC_PI_2);
    assert_eq!(atan2(1.0, 1.0), PI / 4.0);
    assert!(atan2(f64::NAN, 1.0).is_nan());
}
