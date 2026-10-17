//! Evaluation cost per curve kind and per storage shape. Release build, native, single thread.

use geometry_api_spike::fixtures::samples;
use geometry_api_spike::*;
use std::hint::black_box;
use std::time::Instant;

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

/// Median over `reps` runs of nanoseconds per call of `f` over `n` parameters.
fn per_op(reps: usize, params: &[f64], mut f: impl FnMut(f64) -> f64) -> f64 {
    let mut runs: Vec<f64> = (0..reps)
        .map(|_| {
            let start = Instant::now();
            let mut acc = 0.0;
            for &t in params {
                acc += f(black_box(t));
            }
            black_box(acc);
            start.elapsed().as_nanos() as f64 / params.len() as f64
        })
        .collect();
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    runs[reps / 2]
}

fn params(c: &dyn Curve2d, n: usize) -> Vec<f64> {
    let (lo, hi) = c.domain();
    let (lo, hi) = (lo.max(-1_000_000.0), hi.min(1_000_000.0));
    let mut s = 12345u64;
    (0..n).map(|_| lo + (hi - lo) * (0.001 + 0.998 * lcg(&mut s))).collect()
}

fn main() {
    let n = 200_000;
    let reps = 15;
    println!("## Evaluation cost per curve kind (ns per call, median of {reps} runs of {n})\n");
    println!("| kind | eval | derivative (1st) | bounds | transform |");
    println!("|---|---|---|---|---|");
    let m = Affine::new([2.0, 0.7, -0.3, 1.1, 4.0, -5.0]).unwrap();
    for (name, c) in samples() {
        let ps = params(&c, n);
        let ev = per_op(reps, &ps, |t| c.eval(t).map_or(0.0, |p| p[0]));
        let dv = per_op(reps, &ps, |t| c.derivative(t, 1, Side::Right).map_or(0.0, |p| p[0]));
        let small = &ps[..2000];
        let bd = per_op(reps, small, |_| c.bounds().map_or(0.0, |b| b.max[0]));
        let tr = per_op(reps, small, |_| c.transform(&m).map_or(0.0, |(x, _)| x.domain().1));
        println!("| {name} | {ev:.1} | {dv:.1} | {bd:.1} | {tr:.1} |");
    }

    println!(
        "\n## Storage shapes for control points and knots (B-spline, ns per call, median of {reps} runs of {n})\n"
    );
    println!("| curve | shape | eval | build (validate) | build from decoded flat buffer | size_of struct |");
    println!("|---|---|---|---|---|---|");
    for (deg, ncp) in [(3usize, 8usize), (3, 64), (3, 1024), (5, 64)] {
        let interior = ncp - deg - 1;
        let mut knots = vec![0.0; deg + 1];
        knots.extend((1..=interior).map(|j| j as f64 / (interior + 1) as f64));
        knots.extend(vec![1.0; deg + 1]);
        let flat: Vec<f64> = (0..ncp).flat_map(|i| [i as f64, ((i * 7) % 5) as f64]).collect();
        let cps: Vec<Pt> = flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
        let w: Vec<f64> = (0..ncp).map(|i| 1.0 + (i % 3) as f64).collect();
        let ps: Vec<f64> = {
            let mut s = 99u64;
            (0..n).map(|_| 0.001 + 0.998 * lcg(&mut s)).collect()
        };

        let owned = BSpline::new(deg, cps.clone(), knots.clone()).unwrap();
        let borrowed = BSpline::new(deg, Flat(&flat), &knots[..]).unwrap();
        let rowned = RationalBSpline::new(deg, cps.clone(), knots.clone(), w.clone()).unwrap();
        let rborrowed = RationalBSpline::new(deg, Flat(&flat), &knots[..], &w[..]).unwrap();

        let build_reps = if ncp >= 1024 { 300 } else { 5000 };
        let bp: Vec<f64> = vec![0.0; build_reps];
        let b_owned =
            per_op(9, &bp, |_| BSpline::new(deg, cps.clone(), knots.clone()).map_or(0.0, |c| c.domain().1));
        let b_borr =
            per_op(9, &bp, |_| BSpline::new(deg, Flat(&flat), &knots[..]).map_or(0.0, |c| c.domain().1));
        let b_copy = per_op(9, &bp, |_| {
            let v: Vec<Pt> = flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
            BSpline::new(deg, v, knots.clone()).map_or(0.0, |c| c.domain().1)
        });
        let e_o = per_op(reps, &ps, |t| owned.eval(t).map_or(0.0, |p| p[0]));
        let e_b = per_op(reps, &ps, |t| borrowed.eval(t).map_or(0.0, |p| p[0]));
        let r_o = per_op(reps, &ps, |t| rowned.eval(t).map_or(0.0, |p| p[0]));
        let r_b = per_op(reps, &ps, |t| rborrowed.eval(t).map_or(0.0, |p| p[0]));
        println!(
            "| degree {deg}, {ncp} cps | owned Vec | {e_o:.1} | {b_owned:.0} | {b_copy:.0} (copy + validate) | {} |",
            std::mem::size_of_val(&owned)
        );
        println!(
            "| degree {deg}, {ncp} cps | borrowed slices | {e_b:.1} | {b_borr:.0} | n/a (zero copy) | {} |",
            std::mem::size_of_val(&borrowed)
        );
        println!(
            "| degree {deg}, {ncp} cps, rational | owned Vec | {r_o:.1} | | | {} |",
            std::mem::size_of_val(&rowned)
        );
        println!(
            "| degree {deg}, {ncp} cps, rational | borrowed slices | {r_b:.1} | | | {} |",
            std::mem::size_of_val(&rborrowed)
        );
    }

    println!(
        "\n## Dispatch: closed enum vs trait object (ns per call, mixed kinds, median of {reps} runs)\n"
    );
    let curves: Vec<Curve> =
        samples().into_iter().map(|(_, c)| c).filter(|c| !matches!(c, Curve::Line(_))).collect();
    let boxed: Vec<Box<dyn Curve2d>> =
        curves.iter().cloned().map(|c| Box::new(c) as Box<dyn Curve2d>).collect();
    let mut s = 5u64;
    let idx: Vec<f64> = (0..n).map(|_| lcg(&mut s)).collect();
    let k = curves.len();
    let e = per_op(reps, &idx, |u| {
        let c = &curves[(u * k as f64) as usize % k];
        let (lo, hi) = c.domain();
        c.eval(lo + (hi - lo) * 0.37).map_or(0.0, |p| p[0])
    });
    let d = per_op(reps, &idx, |u| {
        let c = &boxed[(u * k as f64) as usize % k];
        let (lo, hi) = c.domain();
        c.eval(lo + (hi - lo) * 0.37).map_or(0.0, |p| p[0])
    });
    println!(
        "| dispatch | domain() + eval() per call |\n|---|---|\n| enum match | {e:.1} |\n| Box<dyn Curve2d> | {d:.1} |"
    );
    println!("\nsize_of::<Curve>() = {} bytes", std::mem::size_of::<Curve>());
}
