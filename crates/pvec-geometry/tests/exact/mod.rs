//! Exact rational arithmetic for the tests of issue 93: a sign-magnitude big integer and a
//! fraction of two of them, with just enough operations to run de Boor without any rounding.
//! Nothing here reduces a fraction, the depth of the recurrences is at most the degree.
#![allow(dead_code)]

use core::cmp::Ordering;

/// Magnitude in base 2^32, little endian, no trailing zero limbs; zero is the empty vector.
#[derive(Clone, Debug)]
pub struct Int {
    neg: bool,
    mag: Vec<u32>,
}

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u64;
    for i in 0..a.len().max(b.len()) {
        let s = *a.get(i).unwrap_or(&0) as u64 + *b.get(i).unwrap_or(&0) as u64 + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry > 0 {
        out.push(carry as u32);
    }
    out
}

/// `a - b` for `a >= b`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (i, &x) in a.iter().enumerate() {
        let mut d = x as i64 - *b.get(i).unwrap_or(&0) as i64 - borrow;
        borrow = 0;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        }
        out.push(d as u32);
    }
    trim(&mut out);
    out
}

fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let cur = out[i + j] as u64 + x as u64 * y as u64 + carry;
            out[i + j] = cur as u32;
            carry = cur >> 32;
        }
        let mut k = i + b.len();
        while carry > 0 {
            let cur = out[k] as u64 + carry;
            out[k] = cur as u32;
            carry = cur >> 32;
            k += 1;
        }
    }
    trim(&mut out);
    out
}

impl Int {
    pub fn zero() -> Int {
        Int { neg: false, mag: Vec::new() }
    }
    pub fn from_u64(x: u64) -> Int {
        let mut mag = vec![x as u32, (x >> 32) as u32];
        trim(&mut mag);
        Int { neg: false, mag }
    }
    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }
    pub fn neg(&self) -> Int {
        Int { neg: !self.neg && !self.is_zero(), mag: self.mag.clone() }
    }
    pub fn shl(&self, bits: usize) -> Int {
        let mut mag = vec![0u32; bits / 32];
        let s = bits % 32;
        let mut carry = 0u32;
        for &l in &self.mag {
            mag.push((l << s) | carry);
            carry = if s == 0 { 0 } else { l >> (32 - s) };
        }
        if carry > 0 {
            mag.push(carry);
        }
        trim(&mut mag);
        Int { neg: self.neg, mag }
    }
    pub fn trailing_zeros(&self) -> usize {
        let mut z = 0;
        for &l in &self.mag {
            if l == 0 {
                z += 32;
            } else {
                return z + l.trailing_zeros() as usize;
            }
        }
        z
    }
    pub fn shr(&self, bits: usize) -> Int {
        let (limbs, s) = (bits / 32, bits % 32);
        let mut mag: Vec<u32> = Vec::new();
        for i in limbs..self.mag.len() {
            let lo = self.mag[i] >> s;
            let hi = if s == 0 { 0 } else { self.mag.get(i + 1).map_or(0, |&x| x << (32 - s)) };
            mag.push(lo | hi);
        }
        trim(&mut mag);
        Int { neg: self.neg && !mag.is_empty(), mag }
    }
    pub fn add(&self, o: &Int) -> Int {
        if self.neg == o.neg {
            return Int { neg: self.neg, mag: add_mag(&self.mag, &o.mag) };
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Equal => Int::zero(),
            Ordering::Greater => Int { neg: self.neg, mag: sub_mag(&self.mag, &o.mag) },
            Ordering::Less => Int { neg: o.neg, mag: sub_mag(&o.mag, &self.mag) },
        }
    }
    pub fn sub(&self, o: &Int) -> Int {
        self.add(&o.neg())
    }
    pub fn mul(&self, o: &Int) -> Int {
        let mag = mul_mag(&self.mag, &o.mag);
        Int { neg: (self.neg != o.neg) && !mag.is_empty(), mag }
    }
    pub fn sign(&self) -> i32 {
        if self.is_zero() {
            0
        } else if self.neg {
            -1
        } else {
            1
        }
    }
    pub fn cmp(&self, o: &Int) -> Ordering {
        match (self.sign(), o.sign()) {
            (a, b) if a != b => a.cmp(&b),
            (0, _) => Ordering::Equal,
            (1, _) => cmp_mag(&self.mag, &o.mag),
            _ => cmp_mag(&o.mag, &self.mag),
        }
    }
}

/// `num / den` with `den > 0`.
#[derive(Clone, Debug)]
pub struct Rat {
    num: Int,
    den: Int,
}

impl Rat {
    /// Divides out the common power of two. The inputs are binary fractions, so this is where the size goes.
    fn norm(self) -> Rat {
        if self.num.is_zero() {
            return Rat { num: Int::zero(), den: Int::from_u64(1) };
        }
        let z = self.num.trailing_zeros().min(self.den.trailing_zeros());
        if z == 0 { self } else { Rat { num: self.num.shr(z), den: self.den.shr(z) } }
    }
    pub fn int(x: u64) -> Rat {
        Rat { num: Int::from_u64(x), den: Int::from_u64(1) }
    }
    /// The exact value of a finite f64.
    pub fn from_f64(x: f64) -> Rat {
        assert!(x.is_finite());
        if x == 0.0 {
            return Rat::int(0);
        }
        let bits = x.to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i64;
        assert!(exp != 0, "subnormal");
        let man = (bits & ((1 << 52) - 1)) | (1 << 52);
        let e = exp - 1075; // x = man * 2^e
        let mut num = Int::from_u64(man);
        if x < 0.0 {
            num = num.neg();
        }
        if e >= 0 {
            Rat { num: num.shl(e as usize), den: Int::from_u64(1) }
        } else {
            Rat { num, den: Int::from_u64(1).shl((-e) as usize) }
        }
    }
    pub fn add(&self, o: &Rat) -> Rat {
        Rat { num: self.num.mul(&o.den).add(&o.num.mul(&self.den)), den: self.den.mul(&o.den) }.norm()
    }
    pub fn sub(&self, o: &Rat) -> Rat {
        Rat { num: self.num.mul(&o.den).sub(&o.num.mul(&self.den)), den: self.den.mul(&o.den) }.norm()
    }
    pub fn mul(&self, o: &Rat) -> Rat {
        Rat { num: self.num.mul(&o.num), den: self.den.mul(&o.den) }.norm()
    }
    pub fn div(&self, o: &Rat) -> Rat {
        assert!(!o.num.is_zero());
        let (n, d) = (self.num.mul(&o.den), self.den.mul(&o.num));
        (if d.sign() < 0 { Rat { num: n.neg(), den: d.neg() } } else { Rat { num: n, den: d } }).norm()
    }
    pub fn is_zero(&self) -> bool {
        self.num.is_zero()
    }
    pub fn cmp(&self, o: &Rat) -> Ordering {
        self.num.mul(&o.den).cmp(&o.num.mul(&self.den))
    }
    pub fn le(&self, o: &Rat) -> bool {
        self.cmp(o) != Ordering::Greater
    }
}

/// de Boor over the rationals for a degree `p` curve with control points `pts` and knots `u`
/// (`u.len() == pts.len() + p + 1`), at `t` inside `[u[p], u[n]]`.
pub fn deboor(p: usize, pts: &[(Rat, Rat)], u: &[Rat], t: &Rat) -> (Rat, Rat) {
    let n = pts.len();
    assert_eq!(u.len(), n + p + 1);
    // the span: u[i] <= t < u[i+1], and for t at the end the last span of positive length
    let mut i = p;
    for j in p..n {
        if u[j].cmp(&u[j + 1]) == Ordering::Less && u[j].le(t) {
            i = j;
            if t.cmp(&u[j + 1]) == Ordering::Less {
                break;
            }
        }
    }
    let mut d: Vec<(Rat, Rat)> = (0..=p).map(|j| pts[j + i - p].clone()).collect();
    let one = Rat::int(1);
    for r in 1..=p {
        for j in (r..=p).rev() {
            let den = u[i + 1 + j - r].sub(&u[i + j - p]);
            let a = if den.is_zero() { Rat::int(0) } else { t.sub(&u[i + j - p]).div(&den) };
            let b = one.sub(&a);
            let x = b.mul(&d[j - 1].0).add(&a.mul(&d[j].0));
            let y = b.mul(&d[j - 1].1).add(&a.mul(&d[j].1));
            d[j] = (x, y);
        }
    }
    d[p].clone()
}

/// `|a - b|^2 <= bound^2`, all exact.
pub fn within(a: &(Rat, Rat), b: &(Rat, Rat), bound: f64) -> bool {
    let (dx, dy) = (a.0.sub(&b.0), a.1.sub(&b.1));
    let bnd = Rat::from_f64(bound);
    dx.mul(&dx).add(&dy.mul(&dy)).le(&bnd.mul(&bnd))
}

/// How many times the bound can be halved before the distance exceeds it: `Some(0)` means the distance is
/// more than half the bound, `Some(64)` that it is below `2^-64` of it, `None` that it exceeds the bound itself.
pub fn halvings(a: &(Rat, Rat), b: &(Rat, Rat), bound: f64) -> Option<u32> {
    let mut bnd = Rat::from_f64(bound);
    if !within(a, b, bound) {
        return None;
    }
    let half = Rat { num: Int::from_u64(1), den: Int::from_u64(2) };
    for k in 0..64 {
        bnd = bnd.mul(&half);
        let (dx, dy) = (a.0.sub(&b.0), a.1.sub(&b.1));
        if !dx.mul(&dx).add(&dy.mul(&dy)).le(&bnd.mul(&bnd)) {
            return Some(k);
        }
    }
    Some(64)
}
