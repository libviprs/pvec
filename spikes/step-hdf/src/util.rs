//! Shared helpers: writer configuration, dataset helpers, I/O accounting, geometry comparison.
#![allow(missing_docs)]

use hdf5::{Group, H5Type};
use hdf5_metno as hdf5;
use ndarray::{ArrayView1, ArrayView2};
use pvec_fixtures::Geom;

/// How datasets are laid out in the file. `chunk` is in rows.
#[derive(Clone, Debug)]
pub struct Cfg {
    pub name: &'static str,
    pub chunk: Option<usize>,
    pub shuffle: bool,
    pub deflate: Option<u8>,
    /// Use the newest file-format versions (compact object headers, v2 B-trees) instead of the oldest.
    pub latest: bool,
}

impl Cfg {
    pub const CONTIGUOUS: Cfg =
        Cfg { name: "contiguous", chunk: None, shuffle: false, deflate: None, latest: false };
}

/// The configurations the spike tries.
pub fn configs() -> Vec<Cfg> {
    let c = |name, chunk, shuffle, deflate, latest| Cfg { name, chunk, shuffle, deflate, latest };
    vec![
        Cfg::CONTIGUOUS,
        c("contiguous+latest", None, false, None, true),
        c("chunk 4096", Some(4096), false, None, false),
        c("chunk 65536", Some(65536), false, None, false),
        c("chunk 4096 + deflate 1", Some(4096), false, Some(1), false),
        c("chunk 4096 + shuffle + deflate 1", Some(4096), true, Some(1), false),
        c("chunk 4096 + shuffle + deflate 6", Some(4096), true, Some(6), false),
        c("chunk 65536 + shuffle + deflate 6", Some(65536), true, Some(6), false),
        c("chunk 65536 + shuffle + deflate 9", Some(65536), true, Some(9), false),
        c("chunk 4096 + shuffle + deflate 6 + latest", Some(4096), true, Some(6), true),
    ]
}

pub fn create_file(path: &str, cfg: &Cfg) -> hdf5::Result<hdf5::File> {
    hdf5::File::with_options().with_fapl(|p| if cfg.latest { p.libver_latest() } else { p }).create(path)
}

/// Writes a 1-D dataset of `T` rows.
pub fn put1<T: H5Type>(g: &Group, name: &str, data: &[T], cfg: &Cfg) -> hdf5::Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    let mut b = g.new_dataset_builder();
    if let Some(c) = cfg.chunk {
        b = b.chunk(c.min(data.len()));
        if cfg.shuffle {
            b = b.shuffle();
        }
        if let Some(l) = cfg.deflate {
            b = b.deflate(l);
        }
    }
    b.with_data(ArrayView1::from(data)).create(name)?;
    Ok(())
}

/// Writes a 2-D `f64` dataset of `data.len() / cols` rows.
pub fn put2(g: &Group, name: &str, cols: usize, data: &[f64], cfg: &Cfg) -> hdf5::Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    let rows = data.len() / cols;
    let view = ArrayView2::from_shape((rows, cols), data).expect("shape");
    let mut b = g.new_dataset_builder();
    if let Some(c) = cfg.chunk {
        b = b.chunk((c.min(rows), cols));
        if cfg.shuffle {
            b = b.shuffle();
        }
        if let Some(l) = cfg.deflate {
            b = b.deflate(l);
        }
    }
    b.with_data(view).create(name)?;
    Ok(())
}

/// Every numeric field of a geometry, in a fixed order, for comparison.
pub fn flatten(g: &Geom, out: &mut Vec<f64>) {
    out.push(g.tag() as f64);
    match g {
        Geom::Line { a, b } => out.extend([a[0], a[1], b[0], b[1]]),
        Geom::Circle { c, r } => out.extend([c[0], c[1], *r]),
        Geom::Arc { c, r, a0, a1 } => out.extend([c[0], c[1], *r, *a0, *a1]),
        Geom::Ellipse { c, a, b, rot, t0, t1 } => out.extend([c[0], c[1], *a, *b, *rot, *t0, *t1]),
        Geom::Polyline { pts } => {
            out.push(pts.len() as f64);
            for p in pts {
                out.extend(*p);
            }
        }
        Geom::Spline { degree, cps, knots, weights } => {
            out.extend([*degree as f64, cps.len() as f64, knots.len() as f64]);
            for p in cps {
                out.extend(*p);
            }
            out.extend(knots);
            if let Some(w) = weights {
                out.extend(w);
            }
        }
        Geom::Composite { parts } => {
            out.push(parts.len() as f64);
            for p in parts {
                flatten(p, out);
            }
        }
    }
}

/// Largest absolute difference between two geometries, or `None` if their shapes differ.
pub fn max_deviation(a: &Geom, b: &Geom) -> Option<f64> {
    let (mut fa, mut fb) = (Vec::new(), Vec::new());
    flatten(a, &mut fa);
    flatten(b, &mut fb);
    if fa.len() != fb.len() {
        return None;
    }
    Some(fa.iter().zip(&fb).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())))
}

/// What the file driver was asked to read.
#[derive(Clone, Debug, Default)]
pub struct Io {
    /// Individual reads the driver performed: (start address, length).
    pub reads: Vec<(u64, u64)>,
}

impl Io {
    /// Parses the log driver's output. Lines look like `  addr-  addr (  n bytes) (flavor) Read`.
    pub fn parse(log: &str) -> Io {
        let mut reads = Vec::new();
        for line in log.lines() {
            if !line.trim_end().ends_with("Read") {
                continue;
            }
            let Some((range, rest)) = line.split_once('(') else { continue };
            let Some(start) = range.trim().split('-').next().and_then(|s| s.trim().parse::<u64>().ok())
            else {
                continue;
            };
            let Some(n) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) else {
                continue;
            };
            reads.push((start, n));
        }
        Io { reads }
    }
    /// Total bytes the driver read, counting a byte once per read.
    pub fn bytes(&self) -> u64 {
        self.reads.iter().map(|r| r.1).sum()
    }
    /// Reads after merging ranges whose gap is at most `gap` bytes, returned as (ranges, bytes in
    /// the merged ranges). With a gap of 0 only touching or overlapping reads merge.
    pub fn merged(&self, gap: u64) -> (usize, u64) {
        let mut v = self.reads.clone();
        v.sort();
        let mut out: Vec<(u64, u64)> = Vec::new();
        for (s, n) in v {
            match out.last_mut() {
                Some(last) if s <= last.1 + gap => last.1 = last.1.max(s + n),
                _ => out.push((s, s + n)),
            }
        }
        (out.len(), out.iter().map(|r| r.1 - r.0).sum())
    }
    /// Highest address touched.
    pub fn max_addr(&self) -> u64 {
        self.reads.iter().map(|r| r.0 + r.1).max().unwrap_or(0)
    }
}

/// Opens a file for reading, with the log driver recording every read if `log` is given.
///
/// `H5_CHUNK_CACHE_MB` raises HDF5's per-dataset chunk cache from its 1 MiB default, so the
/// compressed configurations can be measured both ways.
pub fn open_logged(path: &str, log: Option<&str>) -> hdf5::Result<hdf5::File> {
    let cache_mb: Option<usize> = std::env::var("H5_CHUNK_CACHE_MB").ok().and_then(|v| v.parse().ok());
    hdf5::File::with_options()
        .with_fapl(|p| {
            if let Some(mb) = cache_mb {
                p.chunk_cache(12421, mb * 1024 * 1024, 0.75);
            }
            match log {
                Some(l) => p.log_options(Some(l), hdf5::plist::file_access::LogFlags::LOC_READ, 0),
                None => p,
            }
        })
        .open(path)
}

/// CPU time of this process in seconds. MARS is shared and its load average is far above its core
/// count, so wall-clock timings are inflated by other jobs; CPU time is much less affected.
pub fn cpu_secs() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, writable `timespec` and CLOCK_PROCESS_CPUTIME_ID exists on Linux.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1_000_000_000.0
}

/// Wall and CPU stopwatch.
pub struct Timer(std::time::Instant, f64);

impl Timer {
    pub fn start() -> Timer {
        Timer(std::time::Instant::now(), cpu_secs())
    }
    /// (wall seconds, cpu seconds) since `start`.
    pub fn stop(&self) -> (f64, f64) {
        (self.0.elapsed().as_secs_f64(), cpu_secs() - self.1)
    }
}
