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

const ATAN_HI: [f64; 4] = [
    4.636_476_090_008_060_935_15e-01,
    7.853_981_633_974_482_789_99e-01,
    9.827_937_232_473_290_540_82e-01,
    1.570_796_326_794_896_558_00e+00,
];

const ATAN_LO: [f64; 4] = [
    2.269_877_745_296_168_709_24e-17,
    3.061_616_997_868_383_017_93e-17,
    1.390_331_103_123_099_845_16e-17,
    6.123_233_995_736_766_035_87e-17,
];

const AT: [f64; 11] = [
    3.333_333_333_333_293_180_27e-01,
    -1.999_999_999_987_648_324_76e-01,
    1.428_571_427_250_346_637_11e-01,
    -1.111_111_040_546_235_578_80e-01,
    9.090_887_133_436_506_561_96e-02,
    -7.691_876_205_044_829_994_95e-02,
    6.661_073_137_387_531_206_69e-02,
    -5.833_570_133_790_573_486_45e-02,
    4.976_877_994_615_932_360_17e-02,
    -3.653_157_274_421_691_552_70e-02,
    1.628_582_011_536_578_236_23e-02,
];

const PI: f64 = 3.141_592_653_589_793_116_00;
const PI_LO: f64 = 1.224_646_799_147_353_177_2e-16;

/// The arctangent of `x`, in `(-pi/2, pi/2)`, within one ulp.
pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let negative = x.is_sign_negative();
    let ax = x.abs();
    if ax >= 7.378_697_629_483_820_6e19 {
        // 2^66 and beyond, an infinity included: pi/2 to the last bit
        let r = ATAN_HI[3] + ATAN_LO[3];
        return if negative { -r } else { r };
    }
    let (id, t) = if ax < 0.4375 {
        if ax < 3.725_290_298_461_914e-9 {
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
