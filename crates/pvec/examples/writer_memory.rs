//! Memory and time of the writer at a given number of entities (issue pvec#235), one configuration to a process so
//! that the peak resident set is its own.
//!
//! ```text
//! cargo run --release -p pvec --features unstable-stream --example writer_memory -- <memory|spill> <plain|indexed> <entities> [budget MiB] [spill root]
//! ```
//!
//! `memory` is `PvecWriter::finish`, which keeps the pushes and returns the artifact. `spill` is
//! `with_spill` and `finish_to`, into a sink that only hashes. The entities are two-point polylines, ids from 0, the
//! shape the libviprs harness uses. `indexed` is `new_indexed` with the default leaf and a Tier 0 bound. The line
//! it prints has the peak resident set (Linux `VmHWM`), the artifact's length and SHA-256, and the seconds spent
//! pushing and finishing.
use pvec::unstable::CELL_CAPACITY;
use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter};
use pvec_container::integrity::Sha256;
use pvec_geometry::{Point2, Polyline};
use std::io::Write;
use std::time::Instant;

struct Hash(Sha256, u64);

impl Write for Hash {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        self.1 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn peak_rss_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

fn hex(d: [u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mode, shape) = (a[1].as_str(), a[2].as_str());
    let n: u32 = a[3].parse().expect("entities");
    let budget_mib: u64 = a.get(4).map_or(64, |v| v.parse().expect("budget MiB"));
    let root = a.get(5).cloned().unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());
    let manifest = Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100_000.0, 100_000.0] },
        capabilities: Vec::new(),
        producer_id: "writer-memory".into(),
        producer_version: "1".into(),
    };
    let mut w = match shape {
        "indexed" => PvecWriter::new_indexed(manifest, CELL_CAPACITY, Some(0.05)).unwrap(),
        _ => PvecWriter::new(manifest).unwrap(),
    };
    if mode == "spill" {
        w = w.with_spill(std::path::Path::new(&root), budget_mib << 20).unwrap();
    }
    let started = Instant::now();
    for i in 0..n {
        let x = f64::from(i % 1000) * 10.0;
        let y = f64::from(i / 1000) * 10.0;
        let p = Polyline::new(vec![Point2 { x, y }, Point2 { x: x + 4.0, y: y + 3.0 }]).unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(p)).unwrap();
    }
    let pushed = started.elapsed().as_secs_f64();
    let before = peak_rss_kib();
    let finishing = Instant::now();
    let (len, digest) = if mode == "spill" {
        let mut h = Hash(Sha256::new(), 0);
        let n = w.finish_to(&mut h).unwrap();
        assert_eq!(n, h.1);
        (n, h.0.finalize())
    } else {
        let bytes = w.finish().unwrap();
        (bytes.len() as u64, pvec_container::integrity::sha256(&bytes))
    };
    println!(
        "mode={mode} shape={shape} entities={n} budget_mib={} peak_before_finish_kib={before} peak_kib={} artifact_bytes={len} sha256={} push_s={pushed:.1} finish_s={:.1}",
        if mode == "spill" { budget_mib } else { 0 },
        peak_rss_kib(),
        hex(digest),
        finishing.elapsed().as_secs_f64()
    );
}
