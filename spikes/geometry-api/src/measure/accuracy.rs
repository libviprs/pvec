//! Prints max and mean deviation against closed-form values (acceptance: P1.3).

use core::f64::consts::{FRAC_1_SQRT_2, TAU};
use geometry_api_spike::*;

struct Stat {
    max: f64,
    sum: f64,
    n: usize,
}
impl Stat {
    fn new() -> Self {
        Stat { max: 0.0, sum: 0.0, n: 0 }
    }
    fn add(&mut self, d: f64) {
        self.max = self.max.max(d);
        self.sum += d;
        self.n += 1;
    }
    fn row(&self, name: &str, unit: &str) {
        println!("| {name} | {} | {:.3e} | {:.3e} | {unit} |", self.n, self.max, self.sum / self.n as f64);
    }
}

fn dist(a: Pt, b: Pt) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn main() {
    println!("| check | samples | max deviation | mean deviation | measured as |");
    println!("|---|---|---|---|---|");

    // Closed-form conics: a circle with a rotated frame against c + r(cos(t + phi), sin(t + phi)),
    // an independent expression (angle addition), not the same formula.
    let phi = 0.7f64;
    let c = Circle::new([3.0, -2.0], [phi.cos(), phi.sin()], 7.0).unwrap();
    let mut s = Stat::new();
    for i in 0..100_000 {
        let t = TAU * i as f64 / 100_000.0;
        let want = [3.0 + 7.0 * (t + phi).cos(), -2.0 + 7.0 * (t + phi).sin()];
        s.add(dist(c.eval(t).unwrap(), want) / 7.0);
    }
    s.row("Circle (rotated frame) vs c + r(cos(t+phi), sin(t+phi))", "relative to radius");

    // The exact 9-point rational quadratic NURBS circle against the closed-form circle.
    let w = FRAC_1_SQRT_2;
    let r = 7.0;
    let cps: Vec<Pt> =
        [(1., 0.), (1., 1.), (0., 1.), (-1., 1.), (-1., 0.), (-1., -1.), (0., -1.), (1., -1.), (1., 0.)]
            .iter()
            .map(|&(x, y)| [3.0 + r * x, -2.0 + r * y])
            .collect();
    let weights = vec![1., w, 1., w, 1., w, 1., w, 1.];
    let knots = vec![0., 0., 0., 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1., 1., 1.];
    let nurbs = RationalBSpline::new(2, cps, knots, weights).unwrap();
    let mut s = Stat::new();
    for i in 0..=100_000 {
        let t = i as f64 / 100_000.0;
        let q = nurbs.eval(t).unwrap();
        s.add(((q[0] - 3.0).hypot(q[1] + 2.0) - r).abs() / r);
    }
    s.row("rational quadratic NURBS circle, radial error", "relative to radius");

    // Affine image of an ellipse: transform() against applying the map to evaluated points.
    let e = Ellipse::new([1.0, 2.0], [2.0, 1.0], 5.0, 1.5).unwrap();
    let m = Affine::new([2.0, 0.7, -0.3, 1.1, 4.0, -5.0]).unwrap();
    let (img, pm) = e.transform(&m).unwrap();
    let mut s = Stat::new();
    for i in 0..100_000 {
        let t = TAU * i as f64 / 100_000.0;
        let want = m.point(e.eval(t).unwrap());
        s.add(dist(img.eval(pm.apply(t)).unwrap(), want) / 10.0);
    }
    s.row("Ellipse.transform vs A applied to points", "relative to extent 10");

    // Spline known truth: control points from the blossom of t^p reproduce (t, t^p) exactly.
    for p in [2usize, 3, 5] {
        let n_cp = p + 4;
        let mut knots = vec![0.0; p + 1];
        let interior = n_cp - p - 1;
        for j in 1..=interior {
            // Deliberately non-uniform interior knots.
            knots.push((j as f64 / (interior + 1) as f64).powf(1.3));
        }
        knots.extend(vec![1.0; p + 1]);
        let cps: Vec<Pt> = (0..n_cp)
            .map(|i| {
                let k = &knots[i + 1..i + 1 + p];
                [k.iter().sum::<f64>() / p as f64, k.iter().product::<f64>()]
            })
            .collect();
        let sp = BSpline::new(p, cps.clone(), knots.clone()).unwrap();
        let rat = RationalBSpline::new(p, cps, knots, vec![2.5; n_cp]).unwrap();
        let mut s1 = Stat::new();
        let mut s2 = Stat::new();
        for i in 0..=100_000 {
            let t = i as f64 / 100_000.0;
            let q = sp.eval(t).unwrap();
            s1.add((q[0] - t).abs().max((q[1] - t.powi(p as i32)).abs()));
            let q = rat.eval(t).unwrap();
            s2.add((q[0] - t).abs().max((q[1] - t.powi(p as i32)).abs()));
        }
        s1.row(&format!("B-spline degree {p}, reproduces (t, t^{p})"), "absolute, domain [0,1]");
        s2.row(
            &format!("rational B-spline degree {p}, equal weights 2.5, same truth"),
            "absolute, domain [0,1]",
        );
        // First derivative truth (1, p t^(p-1)).
        let mut s3 = Stat::new();
        for i in 1..100_000 {
            let t = i as f64 / 100_000.0;
            let d = sp.derivative(t, 1, Side::Right).unwrap();
            s3.add((d[0] - 1.0).abs().max((d[1] - p as f64 * t.powi(p as i32 - 1)).abs()));
        }
        s3.row(&format!("B-spline degree {p}, first derivative"), "absolute");
    }

    // Closed-form bounds of a rotated ellipse.
    let e = Ellipse::new([10.0, -4.0], [3.0, 1.0], 5.0, 2.0).unwrap();
    let phi = 1f64.atan2(3.0);
    let hx = ((5.0 * phi.cos()).powi(2) + (2.0 * phi.sin()).powi(2)).sqrt();
    let hy = ((5.0 * phi.sin()).powi(2) + (2.0 * phi.cos()).powi(2)).sqrt();
    let b = e.bounds().unwrap();
    let mut s = Stat::new();
    for d in [b.min[0] - (10.0 - hx), b.max[0] - (10.0 + hx), b.min[1] - (-4.0 - hy), b.max[1] - (-4.0 + hy)]
    {
        s.add(d.abs());
    }
    s.row("full ellipse analytic bounds vs closed form", "absolute");
    // And the same bounds from 64 samples, to show the slack sampling leaves.
    let mut m = [f64::INFINITY, f64::NEG_INFINITY];
    for i in 0..64 {
        let q = e.eval(TAU * i as f64 / 64.0).unwrap();
        m[0] = m[0].min(q[0]);
        m[1] = m[1].max(q[0]);
    }
    println!(
        "\nfor comparison, 64-sample x-bounds of the same ellipse miss the closed form by {:.3e} / {:.3e}",
        m[0] - (10.0 - hx),
        (10.0 + hx) - m[1]
    );
}
