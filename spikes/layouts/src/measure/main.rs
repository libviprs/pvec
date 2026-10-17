//! Measurement driver for P1.5. Subcommands:
//!
//!   measure sizes <out-dir>        encode every fixture with every candidate, write the files, print
//!                                  sizes, cold-open and viewport costs, and decode timings
//!   measure kinds                  bytes per entity by kind on the million fixture
//!   measure mem <cand> <file> [mat]  one decode in a fresh process; prints VmHWM
//!
//! All numbers are only meaningful from a release build on the machine named in the write-up.

use layouts_spike::common::*;
use layouts_spike::fbs_obj::FbObj;
use layouts_spike::fbs_soa::{FbSoa, FbSoaRaw};
use layouts_spike::packed::{self, Packed};
use layouts_spike::rk::Rkyv;
use pvec_fixtures::{Drawing, Entity, generate, p21, viewport_script};
use std::hint::black_box;
use std::time::Instant;

const CANDS: [&str; 5] = ["B", "A", "A-raw", "C", "base"];

macro_rules! with_cand {
    ($name:expr, $c:ident => $body:expr) => {
        match $name {
            "B" => {
                type $c = Packed;
                $body
            }
            "A" => {
                type $c = FbSoa;
                $body
            }
            "A-raw" => {
                type $c = FbSoaRaw;
                $body
            }
            "C" => {
                type $c = Rkyv;
                $body
            }
            "base" => {
                type $c = FbObj;
                $body
            }
            other => panic!("unknown candidate {other}"),
        }
    };
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time_ns(reps: usize, mut f: impl FnMut()) -> f64 {
    f();
    median(
        (0..reps)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_nanos() as f64
            })
            .collect(),
    )
}

fn p21_bytes(d: &Drawing) -> u64 {
    p21::write_part21(d, &mut std::io::sink()).unwrap_or(0)
}

fn fixture(name: &str) -> Drawing {
    match name {
        "small-synthetic" => generate::small_synthetic(),
        "spline-heavy" => generate::spline_heavy(),
        "million" => generate::million(),
        "straddler" => generate::straddler(),
        _ => panic!("unknown fixture"),
    }
}

fn sizes(out_dir: &str) {
    println!("corpus revision: {}\n", pvec_fixtures::REVISION);
    for fx in ["small-synthetic", "spline-heavy", "million", "straddler"] {
        let d = fixture(fx);
        let n = d.entities.len();
        let p21b = p21_bytes(&d);
        let bl = blocks(&d);
        println!(
            "## {fx}: {n} entities, {} blocks of up to {BLOCK}, Part 21 baseline {p21b} bytes\n",
            bl.len()
        );
        println!(
            "| candidate | file bytes | bytes/entity | ratio to Part 21 | encode ms | cold-open bytes (header + directory) | bulk touch ns/entity | viewport-script touch ns/entity | materialise ns/entity |"
        );
        println!("|---|---|---|---|---|---|---|---|---|");
        let mut detail = String::new();
        for c in CANDS {
            with_cand!(c, C => {
                let t = Instant::now();
                let raw = build_file::<C>(&bl);
                let enc_ms = t.elapsed().as_secs_f64() * 1000.0;
                std::fs::write(format!("{out_dir}/{fx}.{c}.bin"), &raw).expect("write");
                let file = Aligned::from_bytes(&raw);
                drop(raw);
                let bytes = file.as_bytes();
                let dir = parse_dir(bytes).expect("dir");
                let (_, cnt) = touch_file::<C>(bytes).expect("touch");
                assert_eq!(cnt as usize, n);
                let bulk = time_ns(7, || {
                    black_box(touch_file::<C>(black_box(bytes)).ok());
                }) / n as f64;
                // The script: touch only the blocks each step hits.
                let script = viewport_script();
                let mut hit_ents = 0u64;
                let steps: Vec<Vec<usize>> = script.iter().map(|s| blocks_hit(&dir, &d.viewport(s))).collect();
                for h in &steps {
                    hit_ents += h.iter().map(|&i| dir[i].nent as u64).sum::<u64>();
                }
                let vp = time_ns(7, || {
                    for h in &steps {
                        for &i in h {
                            black_box(C::touch(block_bytes(bytes, &dir[i])).ok());
                        }
                    }
                }) / hit_ents as f64;
                let mat_reps = if n > 200_000 { 1 } else { 3 };
                let mat = time_ns(mat_reps, || {
                    black_box(decode_file::<C>(bytes).ok());
                }) / n as f64;
                println!(
                    "| {} | {} | {:.1} | {:.4} | {:.0} | {} | {:.1} | {:.1} | {:.1} |",
                    C::NAME,
                    bytes.len(),
                    bytes.len() as f64 / n as f64,
                    bytes.len() as f64 / p21b as f64,
                    enc_ms,
                    HEADER + dir.len() * DIR_ENTRY,
                    bulk,
                    vp,
                    mat
                );
                detail.push_str(&format!("\n### {fx}, {}: viewport script\n\n| step | blocks hit | ranges | bytes read | entities in hit blocks |\n|---|---|---|---|---|\n", C::NAME));
                let (mut tb, mut tr, mut te) = (0u64, 0usize, 0u64);
                for (s, h) in script.iter().zip(&steps) {
                    let r = ranges(&dir, h);
                    let by: u64 = r.iter().map(|x| x.1 - x.0).sum();
                    let ne: u64 = h.iter().map(|&i| dir[i].nent as u64).sum();
                    detail.push_str(&format!("| {} | {} | {} | {} | {} |\n", s.name, h.len(), r.len(), by, ne));
                    tb += by;
                    tr += r.len();
                    te += ne;
                }
                detail.push_str(&format!("| total | | {tr} | {tb} | {te} |\n"));
                if c == "B" {
                    let mut pad = 0usize;
                    for e in &dir {
                        pad += packed::section_padding_bytes(block_bytes(bytes, e));
                    }
                    let kinds = d.kind_counts();
                    let parts: usize = d.entities.iter().map(|e| match &e.geom { pvec_fixtures::Geom::Composite { parts } => parts.len(), _ => 0 }).sum();
                    let comp_pad: usize = d.entities.iter().map(|e| match &e.geom { pvec_fixtures::Geom::Composite { parts } => packed::composite_padding_bytes(parts.len()), _ => 0 }).sum();
                    let _ = parts;
                    detail.push_str(&format!(
                        "\nB padding on {fx}: section alignment and block tail {pad} bytes ({:.2} per entity); spline record header {} bytes ({} per curve x {} curves); composite records {comp_pad} bytes\n",
                        pad as f64 / n as f64,
                        packed::spline_padding_bytes_per_curve() * kinds[5],
                        packed::spline_padding_bytes_per_curve(),
                        kinds[5],
                    ));
                }
            });
        }
        println!("{detail}");
    }
}

fn kinds() {
    let d = generate::million();
    let names = ["line", "circle", "arc", "ellipse", "polyline", "spline", "composite"];
    println!(
        "bytes per entity by kind, million fixture, id-order blocks of {BLOCK}, whole file (header, directory and block overhead included)\n"
    );
    print!("| kind | entities |");
    for c in CANDS {
        with_cand!(c, C => print!(" {} |", C::NAME));
    }
    println!();
    println!("|---|---|{}", "---|".repeat(CANDS.len()));
    for (k, name) in names.iter().enumerate() {
        let ents: Vec<&Entity> = d.entities.iter().filter(|e| e.geom.tag() as usize == k).collect();
        let bl: Vec<Vec<&Entity>> = ents.chunks(BLOCK).map(|c| c.to_vec()).collect();
        print!("| {name} | {} |", ents.len());
        for c in CANDS {
            with_cand!(c, C => print!(" {:.1} |", build_file::<C>(&bl).len() as f64 / ents.len() as f64));
        }
        println!();
    }
}

fn vm_kb(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .map(|l| l.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0))
        })
        .unwrap_or(0)
}

fn mem(cand: &str, path: &str, mat: bool) {
    let raw = std::fs::read(path).expect("read");
    let file = Aligned::from_bytes(&raw);
    let size = raw.len();
    drop(raw);
    let before = vm_kb("VmHWM:");
    let mut result = 0u64;
    with_cand!(cand, C => {
        if mat {
            let v = decode_file::<C>(file.as_bytes()).expect("decode");
            black_box(v.len());
        } else {
            let (sum, n) = touch_file::<C>(file.as_bytes()).expect("touch");
            result = sum.wrapping_add(n);
        }
    });
    let after = vm_kb("VmHWM:");
    println!(
        "{cand} {path} mode={} file={}kB hwm_before_decode={}kB hwm_after={}kB decode_extra={}kB result={result}",
        if mat { "materialise" } else { "touch" },
        size / 1024,
        before,
        after,
        after.saturating_sub(before)
    );
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("sizes") => sizes(a.get(2).map_or("/tmp", String::as_str)),
        Some("kinds") => kinds(),
        Some("mem") => mem(&a[2], &a[3], a.get(4).is_some_and(|s| s == "mat")),
        _ => eprintln!("usage: measure sizes <dir> | kinds | mem <cand> <file> [mat]"),
    }
}
