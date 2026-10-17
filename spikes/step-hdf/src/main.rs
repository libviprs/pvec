//! Measurement driver for P1.4. Every command prints markdown rows; see docs/spikes/01-4-step-hdf.md.
//!
//!   measure sizes    <fixture> <h1|h2> <cfg|all> <outdir>
//!   measure verify   <fixture> <h1|h2> <file>
//!   measure viewport <fixture> <h1|h2> <file>
//!   measure decode   <h1|h2> <file> <expected-checksum-hex>
//!   measure extract  <fixture> <h1|h2> <file>
//!   measure progressive <fixture> <h2|h1> <file>

use pvec_fixtures::{Drawing, Entity, Geom, generate, p21, viewport_script};
use std::time::Instant;
use step_hdf_spike::util::*;
use step_hdf_spike::{h1, h2};

fn fixture(name: &str) -> Drawing {
    match name {
        "small-synthetic" => generate::small_synthetic(),
        "spline-heavy" => generate::spline_heavy(),
        "million" => generate::million(),
        "straddler" => generate::straddler(),
        other => panic!("unknown fixture {other}"),
    }
}

fn p21_bytes(d: &Drawing) -> u64 {
    p21::write_part21(d, &mut std::io::sink()).expect("p21")
}

fn read_all(layout: &str, path: &str) -> Vec<Entity> {
    match layout {
        "h1" => h1::read_all(path).expect("read"),
        _ => h2::read_all(path).expect("read"),
    }
}

fn read_viewport(layout: &str, path: &str, log: Option<&str>, rect: [f64; 4]) -> Vec<Entity> {
    match layout {
        "h1" => h1::read_viewport(path, log, rect).expect("read"),
        _ => h2::read_viewport(path, log, rect).expect("read"),
    }
}

fn storage_sum(path: &str, group: &str) -> u64 {
    let f = hdf5_metno::File::open(path).expect("open");
    let g = f.group(group).expect("group");
    g.datasets().expect("datasets").iter().map(|d| d.storage_size()).sum()
}

fn peak_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
        })
        .unwrap_or(0)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("sizes") => {
            let d = fixture(&a[2]);
            let layout = a[3].as_str();
            let p21b = p21_bytes(&d);
            let payload = if layout == "h1" { h1::payload_bytes(&d) } else { h2::payload_bytes(&d) };
            let group = if layout == "h1" { "data" } else { "pvec_h2" };
            for cfg in configs() {
                if a[4] != "all" && a[4] != cfg.name {
                    continue;
                }
                let path = format!(
                    "{}/{}-{}-{}.h5",
                    a[5],
                    d.name,
                    layout,
                    cfg.name.replace(' ', "_").replace('+', "p")
                );
                let t = Instant::now();
                if layout == "h1" { h1::write(&d, &path, &cfg) } else { h2::write(&d, &path, &cfg) }
                    .expect("write");
                let secs = t.elapsed().as_secs_f64();
                let size = std::fs::metadata(&path).unwrap().len();
                let stored = storage_sum(&path, group);
                println!(
                    "| {} | {} | {} | {} | {:.3} | {} | {:.1}% | {} | {:.1}% | {:.1} |",
                    d.name,
                    layout,
                    cfg.name,
                    size,
                    size as f64 / p21b as f64,
                    payload,
                    100.0 * (size as f64 - payload as f64) / payload as f64,
                    size.saturating_sub(stored),
                    100.0 * size.saturating_sub(stored) as f64 / payload as f64,
                    secs
                );
            }
            println!("part21 bytes: {p21b}; logical payload bytes: {payload}");
        }
        Some("verify") => {
            let d = fixture(&a[2]);
            let got = read_all(&a[3], &a[4]);
            assert_eq!(got.len(), d.entities.len(), "entity count");
            let (mut exact, mut shape_bad, mut worst) = (0usize, 0usize, 0.0f64);
            for (e, g) in d.entities.iter().zip(&got) {
                match max_deviation(&e.geom, &g.geom) {
                    None => shape_bad += 1,
                    Some(0.0) => exact += 1,
                    Some(x) => worst = worst.max(x),
                }
            }
            let rebuilt = Drawing { name: d.name, extent: d.extent, entities: got };
            println!(
                "| {} | {} | {} | {} exact | {} within deviation | {} shape mismatches | worst {:.3e} | checksum match: {} |",
                d.name,
                a[3],
                d.entities.len(),
                exact,
                d.entities.len() - exact - shape_bad,
                shape_bad,
                worst,
                rebuilt.checksum() == d.checksum()
            );
        }
        Some("viewport") => {
            let d = fixture(&a[2]);
            let log = format!("{}.log", a[4]);
            println!(
                "| step | visible | reads | bytes read | ranges (gap 0) | bytes (gap 0) | ranges (gap 64 KiB) | bytes (gap 64 KiB) | wall ms | cpu ms |"
            );
            for s in viewport_script() {
                let rect = d.viewport(&s);
                let expect: Vec<u32> = d
                    .entities
                    .iter()
                    .filter(|e| pvec_fixtures::overlaps(&e.geom.bbox(), &rect))
                    .map(|e| e.id)
                    .collect();
                let _ = std::fs::remove_file(&log);
                let t = Timer::start();
                let got = read_viewport(&a[3], &a[4], Some(&log), rect);
                let (wall, cpu) = t.stop();
                let ids: Vec<u32> = got.iter().map(|e| e.id).collect();
                assert_eq!(ids, expect, "visible set for step {}", s.name);
                for e in &got {
                    let dev = max_deviation(&d.entities[e.id as usize].geom, &e.geom);
                    assert!(
                        dev.is_some_and(|x| x < pvec_numeric::Policy::V1.tol(d.extent[2], d.extent[2])),
                        "entity {} decodes wrongly",
                        e.id
                    );
                }
                let io = Io::parse(&std::fs::read_to_string(&log).unwrap_or_default());
                let (r0, b0) = io.merged(0);
                let (r1, b1) = io.merged(64 * 1024);
                println!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {:.1} | {:.1} |",
                    s.name,
                    got.len(),
                    io.reads.len(),
                    io.bytes(),
                    r0,
                    b0,
                    r1,
                    b1,
                    wall * 1000.0,
                    cpu * 1000.0
                );
            }
        }
        Some("decode") => {
            let before = peak_rss_kb();
            let reps = 3;
            let (mut t, mut tc) = (vec![], vec![]);
            let mut last = None;
            for _ in 0..reps {
                let s = Timer::start();
                let got = read_all(&a[2], &a[3]);
                let (w, c) = s.stop();
                t.push(w);
                tc.push(c);
                last = Some(got);
            }
            let got = last.unwrap();
            let n = got.len();
            let sum = Drawing { name: "decoded", extent: [0.0; 4], entities: got }.checksum();
            let med = median(t);
            let medc = median(tc);
            println!(
                "| {} | {} entities | wall {:.3} s, cpu {:.3} s | {:.0} ns/entity cpu ({:.0} wall) | peak RSS {} KiB (start {} KiB) | checksum {:016x} |",
                a[3],
                n,
                med,
                medc,
                medc * 1_000_000_000.0 / n as f64,
                med * 1_000_000_000.0 / n as f64,
                peak_rss_kb(),
                before,
                sum
            );
            let _ = &a[4];
        }
        Some("extract") => {
            let d = fixture(&a[2]);
            let n_lines = d.entities.iter().filter(|e| matches!(e.geom, Geom::Line { .. })).count();
            let (mut t, mut tc) = (vec![], vec![]);
            let mut out = Vec::new();
            for _ in 0..3 {
                let s = Timer::start();
                out = extract_lines(&a[3], &a[4]);
                let (w, c) = s.stop();
                t.push(w);
                tc.push(c);
            }
            let med = median(tc);
            let _wall = median(t);
            let want: Vec<[f64; 4]> = d
                .entities
                .iter()
                .filter_map(|e| {
                    if let Geom::Line { a, b } = e.geom { Some([a[0], a[1], b[0], b[1]]) } else { None }
                })
                .collect();
            let worst = out
                .iter()
                .zip(&want)
                .fold(0.0f64, |m, (x, y)| (0..4).fold(m, |m, k| m.max((x[k] - y[k]).abs())));
            println!(
                "| {} | {} | {} line segments | {:.1} ms | {:.1} ns/segment | worst deviation {:.3e} | count ok: {} |",
                d.name,
                a[3],
                n_lines,
                med * 1000.0,
                med * 1_000_000_000.0 / n_lines as f64,
                worst,
                out.len() == n_lines
            );
        }
        Some("progressive") => {
            let log = format!("{}.log", a[4]);
            let _ = std::fs::remove_file(&log);
            let size = std::fs::metadata(&a[4]).unwrap().len();
            let f = open_logged(&a[4], Some(&log)).expect("open");
            // Tier 0 stand-in: the cheap kinds. Read their datasets whole and see where the bytes sit.
            let names: &[&str] = if a[3] == "h1" {
                &[
                    "data/GEOMETRIC_CURVE_SET",
                    "data/CARTESIAN_POINT",
                    "data/DIRECTION",
                    "data/VECTOR",
                    "data/LINE",
                    "data/TRIMMED_CURVE",
                    "data/CIRCLE",
                    "data/AXIS2_PLACEMENT_2D",
                    "data/POLYLINE",
                ]
            } else {
                &[
                    "pvec_h2/roots",
                    "pvec_h2/line",
                    "pvec_h2/circle",
                    "pvec_h2/arc",
                    "pvec_h2/poly_off",
                    "pvec_h2/poly_pts",
                ]
            };
            for n in names {
                if let Ok(ds) = f.dataset(n) {
                    match n.rsplit('/').next().unwrap() {
                        "roots" | "GEOMETRIC_CURVE_SET" => {
                            drop(ds.read_raw::<step_hdf_spike::schema::Root>())
                        }
                        "line" | "circle" | "arc" | "poly_pts" => drop(ds.read_raw::<f64>()),
                        "poly_off" => drop(ds.read_raw::<u32>()),
                        "CARTESIAN_POINT" => drop(ds.read_raw::<step_hdf_spike::schema::Point>()),
                        "DIRECTION" => drop(ds.read_raw::<step_hdf_spike::schema::Direction>()),
                        "VECTOR" => drop(ds.read_raw::<step_hdf_spike::schema::Vector>()),
                        "LINE" => drop(ds.read_raw::<step_hdf_spike::schema::LineRow>()),
                        "TRIMMED_CURVE" => drop(ds.read_raw::<step_hdf_spike::schema::TrimRow>()),
                        "CIRCLE" => drop(ds.read_raw::<step_hdf_spike::schema::CircleRow>()),
                        "AXIS2_PLACEMENT_2D" => drop(ds.read_raw::<step_hdf_spike::schema::Placement>()),
                        _ => drop(ds.read_raw::<step_hdf_spike::schema::PolyRow>()),
                    }
                }
            }
            drop(f);
            let io = Io::parse(&std::fs::read_to_string(&log).unwrap_or_default());
            let (r, b) = io.merged(64 * 1024);
            println!(
                "| {} | {} | file {} bytes | tier-0 datasets read: {} reads, {} bytes, {} ranges (gap 64 KiB) | highest address touched {} = {:.1}% of the file |",
                a[3],
                a[4],
                size,
                io.reads.len(),
                io.bytes(),
                r,
                io.max_addr(),
                100.0 * io.max_addr() as f64 / size as f64
            );
            let _ = b;
        }
        Some("hash") => {
            // The same fold the wasm reader computes, so the two can be compared.
            let timer = Timer::start();
            let f = hdf5_metno::File::open(&a[3]).expect("open");
            let mut h = 0xcbf29ce484222325u64;
            let mut mix = |bytes: &[u8]| {
                for b in bytes {
                    h ^= *b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            };
            if a[2] == "h2" {
                for n in [
                    "line",
                    "circle",
                    "arc",
                    "ellipse",
                    "poly_pts",
                    "sp_cps",
                    "sp_knots",
                    "sp_w",
                    "comp_vals",
                    "bbox",
                ] {
                    if let Ok(d) = f.dataset(&format!("pvec_h2/{n}")) {
                        for x in d.read_raw::<f64>().expect("read") {
                            mix(&x.to_bits().to_le_bytes());
                        }
                    }
                }
                for n in ["poly_off", "sp_cp_off", "sp_knot_off", "sp_w_off", "comp_off"] {
                    if let Ok(d) = f.dataset(&format!("pvec_h2/{n}")) {
                        for x in d.read_raw::<u32>().expect("read") {
                            mix(&x.to_le_bytes());
                        }
                    }
                }
            } else {
                if let Ok(d) = f.dataset("data/CARTESIAN_POINT") {
                    for p in d.read_raw::<step_hdf_spike::schema::Point>().expect("read") {
                        mix(&p.x.to_bits().to_le_bytes());
                        mix(&p.y.to_bits().to_le_bytes());
                    }
                }
                if let Ok(d) = f.dataset("data/DIRECTION") {
                    for p in d.read_raw::<step_hdf_spike::schema::Direction>().expect("read") {
                        mix(&p.dx.to_bits().to_le_bytes());
                        mix(&p.dy.to_bits().to_le_bytes());
                    }
                }
                if let Ok(d) = f.dataset("data/POLYLINE") {
                    for p in d.read_raw::<step_hdf_spike::schema::PolyRow>().expect("read") {
                        for r in p.pts.iter() {
                            mix(&r.to_le_bytes());
                        }
                    }
                }
            }
            let (wall, cpu) = timer.stop();
            println!("{h:016x} (native libhdf5: cpu {:.1} ms, wall {:.1} ms)", cpu * 1000.0, wall * 1000.0);
        }
        _ => eprintln!("see the header of src/main.rs"),
    }
}

/// Packed line-segment buffer `[x0, y0, x1, y1]` for every line entity, in entity order.
fn extract_lines(layout: &str, path: &str) -> Vec<[f64; 4]> {
    use step_hdf_spike::schema::*;
    let f = hdf5_metno::File::open(path).expect("open");
    if layout == "h2" {
        let roots: Vec<Root> = f.dataset("pvec_h2/roots").unwrap().read_raw().unwrap();
        let line: Vec<f64> = f.dataset("pvec_h2/line").unwrap().read_raw().unwrap();
        // Line rows were written in entity order, so the packed array is the dataset itself.
        let n = roots.iter().filter(|r| r.tag == 0).count();
        line.chunks_exact(4).take(n).map(|c| [c[0], c[1], c[2], c[3]]).collect()
    } else {
        let roots: Vec<Root> = f.dataset("data/GEOMETRIC_CURVE_SET").unwrap().read_raw().unwrap();
        let trims: Vec<TrimRow> = f.dataset("data/TRIMMED_CURVE").unwrap().read_raw().unwrap();
        let lines: Vec<LineRow> = f.dataset("data/LINE").unwrap().read_raw().unwrap();
        let vecs: Vec<Vector> = f.dataset("data/VECTOR").unwrap().read_raw().unwrap();
        let dirs: Vec<Direction> = f.dataset("data/DIRECTION").unwrap().read_raw().unwrap();
        let pts: Vec<Point> = f.dataset("data/CARTESIAN_POINT").unwrap().read_raw().unwrap();
        let mut out = Vec::new();
        for r in roots.iter().filter(|r| r.tag == tag::TRIM) {
            let t = &trims[r.row as usize];
            if t.basis_tag != tag::LINE {
                continue;
            }
            let l = &lines[t.basis as usize];
            let (v, p) = (&vecs[l.vec as usize], &pts[l.pnt as usize]);
            let d = &dirs[v.dir as usize];
            let (x, y) = (v.mag * d.dx, v.mag * d.dy);
            out.push([p.x + t.t1 * x, p.y + t.t1 * y, p.x + t.t2 * x, p.y + t.t2 * y]);
        }
        out
    }
}
