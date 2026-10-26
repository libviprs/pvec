//! P8.2: the STEP-HDF arm over the benchmark corpus. The corpus half of the harness (`crates/pvec-bench`) writes
//! a shape dump (`PVSH1`, described in `crates/pvec-bench/src/dump.rs`); this module turns it into the phase 1
//! `Drawing`, writes it with the phase 1 H2 layout in the three configurations the comparison uses, and measures
//! the viewport reads with the phase 1 methods (a fresh open per viewport, the log file driver for the bytes).
//!
//! Each command prints nothing and writes a `key=value` file, because the peak resident set is the process's
//! and every measurement therefore runs in a process of its own.

use crate::h2;
use crate::util::{Cfg, Io, Timer, configs, open_logged};
use pvec_fixtures::{Drawing, Entity, Geom};
use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::path::Path;

const MAGIC: &[u8; 8] = b"PVSH1\0\0\0";
/// The three configurations: the plain one, and the two compressed ones the comparison takes the smaller of.
pub const CONFIGS: [&str; 3] =
    ["contiguous", "chunk 65536 + shuffle + deflate 6", "chunk 4096 + shuffle + deflate 6 + latest"];

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

struct Reader<'a>(&'a [u8], usize);

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> &[u8] {
        let s = &self.0[self.1..self.1 + n];
        self.1 += n;
        s
    }
    fn u8(&mut self) -> u8 {
        self.bytes(1)[0]
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes(4).try_into().expect("4 bytes"))
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.bytes(8).try_into().expect("8 bytes"))
    }
    fn f(&mut self) -> f64 {
        f64::from_bits(self.u64())
    }
    fn fs(&mut self, n: usize) -> Vec<f64> {
        (0..n).map(|_| self.f()).collect()
    }
}

/// The drawing in a shape dump. Fails if the digest in the header is not the digest of the body.
pub fn load(name: &str, bytes: &[u8]) -> Result<Drawing, String> {
    if bytes.len() < 56 || &bytes[..8] != MAGIC {
        return Err("not a PVSH1 dump".into());
    }
    let mut h = Reader(bytes, 8);
    let count = h.u64() as usize;
    let digest = h.u64();
    let extent = [h.f(), h.f(), h.f(), h.f()];
    if fnv(&bytes[h.1..]) != digest {
        return Err("the digest in the header is not the digest of the body".into());
    }
    let mut entities = Vec::with_capacity(count);
    for id in 0..count {
        let geom = match h.u8() {
            0 => Geom::Line { a: [h.f(), h.f()], b: [h.f(), h.f()] },
            1 => {
                let n = h.u32() as usize;
                Geom::Polyline { pts: (0..n).map(|_| [h.f(), h.f()]).collect() }
            }
            2 => {
                let (c, r, start, sweep) = ([h.f(), h.f()], h.f(), h.f(), h.f());
                if sweep.abs() == TAU {
                    Geom::Circle { c, r }
                } else {
                    let (a0, a1) = if sweep > 0.0 { (start, start + sweep) } else { (start + sweep, start) };
                    Geom::Arc { c, r, a0, a1 }
                }
            }
            3 => {
                let (c, a, b, rot, start, sweep) = ([h.f(), h.f()], h.f(), h.f(), h.f(), h.f(), h.f());
                let (t0, t1) = if sweep.abs() == TAU {
                    (0.0, TAU)
                } else if sweep > 0.0 {
                    (start, start + sweep)
                } else {
                    (start + sweep, start)
                };
                Geom::Ellipse { c, a, b, rot, t0, t1 }
            }
            4 => {
                let (degree, weighted) = (h.u8(), h.u8() == 1);
                let (n, k) = (h.u32() as usize, h.u32() as usize);
                let (x, y, knots) = (h.fs(n), h.fs(n), h.fs(k));
                let weights = weighted.then(|| h.fs(n));
                Geom::Spline {
                    degree,
                    cps: x.iter().zip(&y).map(|(x, y)| [*x, *y]).collect(),
                    knots,
                    weights,
                }
            }
            t => return Err(format!("unknown tag {t}")),
        };
        entities.push(Entity { id: id as u32, geom });
    }
    Ok(Drawing { name: Box::leak(name.to_string().into_boxed_str()), extent, entities })
}

fn cfg(name: &str) -> Cfg {
    configs().into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no configuration {name}"))
}

fn tag(name: &str) -> String {
    name.replace(' ', "_").replace('+', "p")
}

fn write_kv(path: &Path, kv: &BTreeMap<String, String>) {
    let text: String = kv.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    std::fs::write(path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn read_kv(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn percentile(v: &[f64], q: f64) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((q * v.len() as f64).ceil() as usize).max(1) - 1]
}

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// `corpus-write ENTRY DUMP OUT`: writes the three files and checks that the layout hands the drawing back bit
/// for bit. The sizes go to `ENTRY.hdfw.kv`.
pub fn write(entry: &str, dump: &Path, out: &Path) {
    let d = load(entry, &std::fs::read(dump).expect("the dump")).unwrap_or_else(|e| panic!("{entry}: {e}"));
    let mut kv = BTreeMap::new();
    let want = d.checksum();
    for name in CONFIGS {
        let path = out.join(format!("{entry}.{}.h5", tag(name)));
        let t = Timer::start();
        h2::write(&d, path.to_str().expect("utf8"), &cfg(name)).expect("write");
        let (wall, _) = t.stop();
        let got = Drawing {
            name: d.name,
            extent: d.extent,
            entities: h2::read_all(path.to_str().expect("utf8")).expect("read"),
        };
        let exact = got.checksum() == want && got.entities.len() == d.entities.len();
        kv.insert(format!("bytes {name}"), std::fs::metadata(&path).expect("file").len().to_string());
        kv.insert(format!("write_s {name}"), wall.to_string());
        kv.insert(format!("roundtrip_exact {name}"), exact.to_string());
    }
    kv.insert("entities".into(), d.entities.len().to_string());
    write_kv(&out.join(format!("{entry}.hdfw.kv")), &kv);
}

/// `corpus-read ENTRY OUT VIEWPORTS CONFIG`: the cold open and the 200 recorded viewports against one file.
/// `VIEWPORTS` is a text file of `x0,y0,x1,y1` lines. The chunk cache is 256 MiB (the phase 1 spike's
/// finding is that the default makes a compressed file unusable), set through `H5_CHUNK_CACHE_MB`.
pub fn read(entry: &str, out: &Path, viewports: &Path, config: &str) {
    let path = out.join(format!("{entry}.{}.h5", tag(config)));
    let p = path.to_str().expect("utf8");
    let rects: Vec<[f64; 4]> = std::fs::read_to_string(viewports)
        .expect("viewports")
        .lines()
        .map(|l| {
            let v: Vec<f64> = l.split(',').map(|x| x.parse().expect("a number")).collect();
            [v[0], v[1], v[2], v[3]]
        })
        .collect();
    assert_eq!(rects.len(), 200, "200 recorded viewports");
    // Directory ready: the file open and the two datasets every query starts from.
    let mut opens = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let f = open_logged(p, None).expect("open");
        let _roots: Vec<crate::schema::Root> =
            f.dataset("pvec_h2/roots").expect("roots").read_raw().expect("read");
        let _bbox: Vec<f64> = f.dataset("pvec_h2/bbox").expect("bbox").read_raw().expect("read");
        opens.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let (mut times, mut bytes) = (Vec::new(), Vec::new());
    let log = format!("{p}.log");
    for r in &rects {
        let t = std::time::Instant::now();
        let got = h2::read_viewport(p, None, *r).expect("viewport");
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        drop(got);
        let _ = std::fs::remove_file(&log);
        drop(h2::read_viewport(p, Some(&log), *r).expect("viewport"));
        bytes.push(Io::parse(&std::fs::read_to_string(&log).unwrap_or_default()).bytes() as f64);
    }
    let _ = std::fs::remove_file(&log);
    let mut kv = BTreeMap::new();
    kv.insert("cold_open_ms".into(), percentile(&opens, 0.5).to_string());
    kv.insert("viewport_p50_ms".into(), percentile(&times, 0.5).to_string());
    kv.insert("viewport_p95_ms".into(), percentile(&times, 0.95).to_string());
    kv.insert("viewport_bytes_max".into(), percentile(&bytes, 1.0).to_string());
    kv.insert("peak_rss_kib".into(), peak_rss_kib().to_string());
    write_kv(&out.join(format!("{entry}.hdfr.{}.kv", tag(config))), &kv);
}

/// `corpus-merge ENTRY OUT`: the sizes, and the viewport numbers of the configuration with the smallest p95.
/// The compressed size is the smaller of the two compressed files. Both choices favour STEP-HDF, which is the
/// strict way to judge `.pvec` against it, and the note lists every configuration.
pub fn merge(entry: &str, out: &Path) {
    let w = read_kv(&out.join(format!("{entry}.hdfw.kv")));
    let size = |name: &str| w[&format!("bytes {name}")].parse::<f64>().expect("a size");
    let mut kv = BTreeMap::new();
    kv.insert("contiguous_bytes".into(), size(CONFIGS[0]).to_string());
    kv.insert("compressed_bytes".into(), size(CONFIGS[1]).min(size(CONFIGS[2])).to_string());
    let reads: Vec<(&str, BTreeMap<String, String>)> =
        CONFIGS.iter().map(|c| (*c, read_kv(&out.join(format!("{entry}.hdfr.{}.kv", tag(c)))))).collect();
    let p95 = |kv: &BTreeMap<String, String>| kv["viewport_p95_ms"].parse::<f64>().expect("p95");
    let (best_name, best) = reads.iter().min_by(|a, b| p95(&a.1).total_cmp(&p95(&b.1))).expect("three");
    for key in ["cold_open_ms", "viewport_p50_ms", "viewport_p95_ms", "viewport_bytes_max", "peak_rss_kib"] {
        kv.insert(key.into(), best[key].clone());
    }
    let exact = CONFIGS.iter().all(|c| w[&format!("roundtrip_exact {c}")] == "true");
    let per: Vec<String> = reads.iter().map(|(n, k)| format!("{n}: p95 {:.3} ms", p95(k))).collect();
    kv.insert(
        "note".into(),
        format!(
            "H2 layout of the phase 1 spike; viewport numbers from the configuration with the best p95 ({best_name}) with a 256 MiB chunk cache; all three: {}; compressed is the smaller of the two compressed files; the layout has no spatial index so every viewport scans the bounds table; round trip bit exact: {exact}",
            per.join(" / ")
        ),
    );
    write_kv(&out.join(format!("{entry}.hdf.kv")), &kv);
}
