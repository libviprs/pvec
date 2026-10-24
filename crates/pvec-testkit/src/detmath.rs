//! Arctangent from basic arithmetic only (issue #101).
//!
//! The standard library's `atan` and `atan2` call the C library of the machine, and the last bit of
//! what they return belongs to that library, not to PVec: the same call gives different bits on
//! glibc, musl and the system library of a Mac. A number stored in an artifact has to be a function
//! of the inputs alone, so the generator takes its angles from here instead.
//!
//! This is the algorithm of fdlibm (`s_atan.c`, `e_atan2.c`), whose result is within one unit in the
//! last place, written with `+ - * /` and comparisons and nothing else. Those operations are
//! correctly rounded by IEEE 754 and Rust never fuses a multiply and an add, so the bits are the same
//! on every target. The constants are the published fdlibm ones, spelled out to the digits fdlibm
//! gives. A test pins two results bit for bit, which is what runs on x64 and arm64.
//!
//! The functions take finite arguments. A NaN comes back as a NaN, and an infinity as the limit.

// The fdlibm constants as the bit patterns of the doubles they denote, with the published decimal
// beside each one. A bit pattern cannot be rounded differently by a compiler or a lint.

/// atan(0.5), atan(1), atan(1.5) and atan(inf), high parts.
const ATAN_HI: [f64; 4] = [
    f64::from_bits(0x3fdd_ac67_0561_bb4f), // 4.63647609000806093515e-01
    f64::from_bits(0x3fe9_21fb_5444_2d18), // 7.85398163397448278999e-01
    f64::from_bits(0x3fef_730b_d281_f69b), // 9.82793723247329054082e-01
    f64::from_bits(0x3ff9_21fb_5444_2d18), // 1.57079632679489655800e+00
];

/// The matching low parts.
const ATAN_LO: [f64; 4] = [
    f64::from_bits(0x3c7a_2b7f_222f_65e2), // 2.26987774529616870924e-17
    f64::from_bits(0x3c81_a626_3314_5c07), // 3.06161699786838301793e-17
    f64::from_bits(0x3c70_0788_7af0_cbbd), // 1.39033110312309984516e-17
    f64::from_bits(0x3c91_a626_3314_5c07), // 6.12323399573676603587e-17
];

/// The coefficients of the polynomial for atan on the reduced argument.
const AT: [f64; 11] = [
    f64::from_bits(0x3fd5_5555_5555_550d), // 3.33333333333329318027e-01
    f64::from_bits(0xbfc9_9999_9998_ebc4), // -1.99999999998764832476e-01
    f64::from_bits(0x3fc2_4924_9200_83ff), // 1.42857142725034663711e-01
    f64::from_bits(0xbfbc_71c6_fe23_1671), // -1.11111104054623557880e-01
    f64::from_bits(0x3fb7_45cd_c54c_206e), // 9.09088713343650656196e-02
    f64::from_bits(0xbfb3_b0f2_af74_9a6d), // -7.69187620504482999495e-02
    f64::from_bits(0x3fb1_0d66_a0d0_3d51), // 6.66107313738753120669e-02
    f64::from_bits(0xbfad_de2d_52de_fd9a), // -5.83357013379057348645e-02
    f64::from_bits(0x3fa9_7b4b_2476_0deb), // 4.97687799461593236017e-02
    f64::from_bits(0xbfa2_b444_2c6a_6c2f), // -3.65315727442169155270e-02
    f64::from_bits(0x3f90_ad3a_e322_da11), // 1.62858201153657823623e-02
];

const PI: f64 = f64::from_bits(0x4009_21fb_5444_2d18); // 3.1415926535897931160E+00
const PI_LO: f64 = f64::from_bits(0x3ca1_a626_3314_5c07); // 1.2246467991473531772E-16
/// 2^66: from here atan is pi/2 to the last bit.
const HUGE: f64 = f64::from_bits(0x4410000000000000);
/// 2^-28: below this atan(x) is x to the last bit.
const TINY: f64 = f64::from_bits(0x3e30000000000000);

/// The arctangent of `x`, in `(-pi/2, pi/2)`, within one ulp.
pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let negative = x.is_sign_negative();
    let ax = x.abs();
    if ax >= HUGE {
        // an infinity included: pi/2 to the last bit
        let r = ATAN_HI[3] + ATAN_LO[3];
        return if negative { -r } else { r };
    }
    let (id, t) = if ax < 0.4375 {
        if ax < TINY {
            // 2^-28: atan(x) is x to the last bit
            return x;
        }
        (None, ax)
    } else if ax < 1.1875 {
        if ax < 0.6875 {
            (Some(0), (2.0 * ax - 1.0) / (2.0 + ax))
        } else {
            (Some(1), (ax - 1.0) / (ax + 1.0))
        }
    } else if ax < 2.4375 {
        (Some(2), (ax - 1.5) / (1.0 + 1.5 * ax))
    } else {
        (Some(3), -1.0 / ax)
    };
    let z = t * t;
    let w = z * z;
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    let r = match id {
        None => t - t * (s1 + s2),
        Some(i) => ATAN_HI[i] - ((t * (s1 + s2) - ATAN_LO[i]) - t),
    };
    if negative { -r } else { r }
}

/// The angle of the point `(x, y)` from the positive x axis, in `[-pi, pi]`, within one ulp. The sign
/// of a zero `y` is kept: `atan2(-0.0, -1.0)` is `-pi`.
pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let (xneg, yneg) = (x.is_sign_negative(), y.is_sign_negative());
    // quadrant code as fdlibm has it: bit 1 is the sign of x, bit 0 the sign of y
    let m = (u8::from(xneg) << 1) | u8::from(yneg);
    if y == 0.0 {
        return match m {
            0 | 1 => y,
            2 => PI,
            _ => -PI,
        };
    }
    if x == 0.0 {
        return if yneg { -ATAN_HI[3] - ATAN_LO[3] } else { ATAN_HI[3] + ATAN_LO[3] };
    }
    if x.is_infinite() {
        return if y.is_infinite() {
            match m {
                0 => PI / 4.0,
                1 => -PI / 4.0,
                2 => 3.0 * PI / 4.0,
                _ => -3.0 * PI / 4.0,
            }
        } else {
            match m {
                0 => 0.0,
                1 => -0.0,
                2 => PI,
                _ => -PI,
            }
        };
    }
    if y.is_infinite() {
        return if yneg { -ATAN_HI[3] - ATAN_LO[3] } else { ATAN_HI[3] + ATAN_LO[3] };
    }
    let (ax, ay) = (x.abs(), y.abs());
    // the exponent gap decides when the quotient is hopeless
    let k = exponent(ay) - exponent(ax);
    let (z, m) = if k > 60 {
        (ATAN_HI[3] + 0.5 * PI_LO, m & 1)
    } else if xneg && k < -60 {
        (0.0, m)
    } else {
        (atan(ay / ax), m)
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

/// The biased exponent field of a finite value, which is all fdlibm's `(iy - ix) >> 20` reads.
fn exponent(v: f64) -> i32 {
    i32::try_from((v.to_bits() >> 52) & 0x7ff).unwrap_or(0)
}
