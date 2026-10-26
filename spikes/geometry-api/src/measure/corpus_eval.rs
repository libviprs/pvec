//! P8.4: the P1.3 prototype's `eval` on the reference cubic of `crates/pvec-bench/benches/geometry.rs`, measured
//! the way `bench.rs` measures it (median of 15 runs over 200,000 parameters from the same generator), so the
//! production number has a baseline on the same curve and the same parameters.
//!
//! The curve: degree 3, twelve control points `(10 i, 3 ((37 i) mod 11))`, a clamped knot vector with nine equal
//! spans. Output is `key=value` lines.

use geometry_api_spike::*;
use std::hint::black_box;
use std::time::Instant;

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

fn main() {
    let cps: Vec<Pt> = (0..12).map(|i| [10.0 * i as f64, 3.0 * ((37 * i) % 11) as f64]).collect();
    let mut knots = vec![0.0; 4];
    knots.extend((1..9).map(|k| k as f64 / 9.0));
    knots.extend(vec![1.0; 4]);
    let c = BSpline::new(3, cps, knots).unwrap();
    let (lo, hi) = c.domain();
    let mut s = 12345u64;
    let ps: Vec<f64> = (0..200_000).map(|_| lo + (hi - lo) * (0.001 + 0.998 * lcg(&mut s))).collect();
    let mut runs: Vec<f64> = (0..15)
        .map(|_| {
            let start = Instant::now();
            let mut acc = 0.0;
            for &t in &ps {
                acc += c.eval(black_box(t)).map_or(0.0, |p| p[0]);
            }
            black_box(acc);
            start.elapsed().as_nanos() as f64 / ps.len() as f64
        })
        .collect();
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("prototype_point_at_ns={}", runs[7]);
}
