//! Issue #205: opening a file by byte ranges the host supplies. A `Host` here plays the browser: it owns
//! the bytes, answers what the module says it needs, and counts every request. `docs/WASM_ABI.md` is the
//! contract; every test name carries the issue number.

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter, Transform2};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::{MemorySource, Recording};
use pvec_geometry::{BSpline, Point2, Polyline};
use pvec_wasm::{AbiError, Precision, Runtime, WASM_ABI_VERSION};
use std::fs;
use std::path::Path;

const SHORT: u32 = 3000;
const WAVES: u32 = 40;

fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

/// 3000 short polylines on a 60 x 50 grid, 40 waves of 200 points across it, two splines (the advanced
/// curves), in leaves of 32 with a Tier 0 block each.
fn file() -> Vec<u8> {
    let manifest = Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [1000.0, 1000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-205".into(),
        producer_version: "1".into(),
    };
    let mut w = PvecWriter::new_indexed(manifest, 32, Some(0.05)).unwrap();
    for i in 0..SHORT {
        let (x, y) = (f64::from(i % 60) * 16.0, f64::from(i / 60) * 19.0);
        w.push(id(i), &Curve::Polyline(Polyline::new(vec![p(x, y), p(x + 6.0, y + 4.0)]).unwrap())).unwrap();
    }
    for k in 0..WAVES {
        let y0 = 20.0 + f64::from(k) * 24.0;
        let pts = (0..200).map(|j| p(f64::from(j) * 4.5, y0 + (f64::from(j) * 0.2).sin() * 6.0)).collect();
        w.push(id(SHORT + k), &Curve::Polyline(Polyline::new(pts).unwrap())).unwrap();
    }
    for (i, dy) in [(SHORT + WAVES, 0.0), (SHORT + WAVES + 1, 5.0)] {
        let s = BSpline::new(
            2,
            vec![p(10.0, 900.0 + dy), p(60.0, 960.0 + dy), p(120.0, 900.0 + dy)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        )
        .unwrap();
        w.push(id(i), &Curve::BSpline(s)).unwrap();
    }
    w.finish().unwrap()
}

fn window(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds2 {
    Bounds2 { min: [x0, y0], max: [x1, y1] }
}

fn everything() -> Bounds2 {
    window(-1_000_000_000.0, -1_000_000_000.0, 1_000_000_000.0, 1_000_000_000.0)
}

/// The browser: bytes, and a log of what it was asked for.
struct Host {
    bytes: Vec<u8>,
    served: Vec<(u64, u32)>,
    rounds: u32,
}

impl Host {
    fn new(bytes: Vec<u8>) -> Host {
        Host { bytes, served: Vec::new(), rounds: 0 }
    }

    fn serve(&mut self, rt: &mut Runtime, handle: u32) {
        self.rounds += 1;
        for (offset, len) in rt.needed(handle).unwrap() {
            self.served.push((offset, len));
            let end = (offset + u64::from(len)) as usize;
            rt.provide_range(handle, offset, &self.bytes[offset as usize..end]).unwrap();
        }
    }

    fn open(&mut self, rt: &mut Runtime) -> u32 {
        let h = rt.open_ranged(self.bytes.len() as u64).unwrap();
        loop {
            match rt.open_poll(h) {
                Ok(f) => return f,
                Err(AbiError::NeedRange) => self.serve(rt, h),
                Err(e) => panic!("open failed: {e:?}"),
            }
        }
    }

    /// Runs `call` until it stops asking for ranges.
    fn run<T>(
        &mut self,
        rt: &mut Runtime,
        handle: u32,
        mut call: impl FnMut(&mut Runtime) -> Result<T, AbiError>,
    ) -> Result<T, AbiError> {
        loop {
            match call(rt) {
                Err(AbiError::NeedRange) => self.serve(rt, handle),
                other => return other,
            }
        }
    }

    fn bytes_served(&self) -> u64 {
        self.served.iter().map(|(_, l)| u64::from(*l)).sum()
    }
}

#[test]
fn issue_205_the_abi_stays_2_and_the_document_names_the_range_calls() {
    assert_eq!(WASM_ABI_VERSION, 2);
    assert_eq!(AbiError::NeedRange.code(), -7);
    let doc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/WASM_ABI.md")).unwrap();
    for needle in
        ["open_ranged", "provide_range", "open_poll", "needed_count", "needed_offset", "needed_len", "-7"]
    {
        assert!(doc.contains(needle), "docs/WASM_ABI.md does not mention {needle}");
    }
}

#[test]
fn issue_205_opening_costs_what_the_container_open_costs_plus_the_spatial_directory_and_reads_no_entity() {
    let bytes = file();
    let reader =
        ContainerReader::open(Recording::new(MemorySource::new(bytes.clone())), ReaderConfig::default())
            .unwrap();
    // the requests the container's own open makes: the header prefix, then the table and the directory in one
    let (baseline, baseline_bytes) = (reader.source().requests().len(), reader.source().bytes_requested());
    let spatial: Vec<_> = reader
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::SpatialDirectory))
        .map(|e| u64::from(e.stored_length))
        .collect();
    let mut host = Host::new(bytes);
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    assert_eq!(host.served.len(), baseline + 1, "{:?}", host.served);
    assert_eq!(host.bytes_served(), baseline_bytes + spatial[0], "{:?}", host.served);
    let info = rt.directory(f).unwrap().expect("the directory is there without another request");
    assert!(info.cells > 50);
    assert_eq!(rt.query_viewport(f, window(0.0, 0.0, 1.0, 1.0), 10).map(|_| ()), Err(AbiError::NeedRange));
    assert!(!rt.needed(f).unwrap().is_empty(), "an entity read is a separate, later request");
}

#[test]
fn issue_205_the_directory_calls_need_no_further_request_after_open() {
    let mut host = Host::new(file());
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    let after_open = host.served.len();
    let q = rt.directory_query(f, window(0.0, 0.0, 100.0, 100.0)).unwrap();
    assert!(!rt.cells(q).unwrap().ids.is_empty());
    assert_eq!(host.served.len(), after_open);
}

#[test]
fn issue_205_directory_and_tier0_answers_equal_the_in_memory_ones() {
    let bytes = file();
    let mut mem = Runtime::new();
    let m = mem.open(bytes.clone()).unwrap();
    let mut host = Host::new(bytes);
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    assert_eq!(rt.directory(f).unwrap(), mem.directory(m).unwrap());
    let windows = [
        window(0.0, 0.0, 120.0, 120.0),
        window(300.0, 200.0, 640.0, 410.0),
        window(-5.0, 880.0, 200.0, 990.0),
        window(500.0, 500.0, 500.5, 500.5),
        everything(),
        window(5_000_000.0, 5_000_000.0, 6_000_000.0, 6_000_000.0),
    ];
    for w in windows {
        let a = mem.directory_query(m, w).unwrap();
        let b = host.run(&mut rt, f, |r| r.directory_query(f, w)).unwrap();
        assert_eq!(rt.cells(b).unwrap(), mem.cells(a).unwrap(), "cells of {w:?}");
        let a = mem.tier0_viewport(m, w).unwrap();
        let b = host.run(&mut rt, f, |r| r.tier0_viewport(f, w)).unwrap();
        assert_eq!(rt.tier0(b).unwrap(), mem.tier0(a).unwrap(), "tier 0 of {w:?}");
    }
}

#[test]
fn issue_205_a_viewport_query_and_its_tessellation_equal_the_in_memory_ones() {
    let bytes = file();
    let mut mem = Runtime::new();
    let m = mem.open(bytes.clone()).unwrap();
    let mut host = Host::new(bytes);
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    let ctx = pvec::TessellationContext {
        source_to_map: Transform2::IDENTITY,
        map_to_screen: Transform2::scale(2.0, 2.0).unwrap(),
        pixel_ratio: 1.0,
        max_screen_error_px: 0.25,
        max_world_error: f64::INFINITY,
        max_segments: 4096,
    };
    for w in [window(0.0, 0.0, 120.0, 120.0), window(-5.0, 880.0, 200.0, 990.0), everything()] {
        let a = mem.query_viewport(m, w, u32::MAX).unwrap();
        let b = host.run(&mut rt, f, |r| r.query_viewport(f, w, u32::MAX)).unwrap();
        assert_eq!(rt.batch_len(b).unwrap(), mem.batch_len(a).unwrap(), "{w:?}");
        let ta = mem.tessellate_batch(a, &ctx, Precision::F64).unwrap();
        let tb = rt.tessellate_batch(b, &ctx, Precision::F64).unwrap();
        assert_eq!(rt.entity_ids(tb).unwrap(), mem.entity_ids(ta).unwrap(), "{w:?}");
        assert_eq!(rt.offsets(tb).unwrap(), mem.offsets(ta).unwrap());
        assert_eq!(rt.vertices_f64(tb).unwrap(), mem.vertices_f64(ta).unwrap());
        assert_eq!(rt.style_refs(tb).unwrap(), mem.style_refs(ta).unwrap());
    }
}

#[test]
fn issue_205_a_small_window_asks_for_few_ranges_in_one_round_and_a_repeat_asks_for_none() {
    let mut host = Host::new(file());
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    let (opened, rounds, opened_bytes) = (host.served.len(), host.rounds, host.bytes_served());
    let w = window(100.0, 100.0, 220.0, 200.0);
    let t = host.run(&mut rt, f, |r| r.tier0_viewport(f, w)).unwrap();
    assert!(rt.tier0(t).unwrap().ids.len() > 10);
    assert_eq!(host.rounds, rounds + 1, "every range the window needs is named at once");
    let asked = host.served.len() - opened;
    let cells = rt.directory_query(f, w).unwrap();
    let touched = rt.cells(cells).unwrap().ids.len();
    assert!(asked >= 1 && asked <= touched, "{asked} requests for {touched} leaves");
    assert!((host.bytes_served() - opened_bytes) * 3 < host.bytes.len() as u64);
    let before = host.served.len();
    let again = host.run(&mut rt, f, |r| r.tier0_viewport(f, w)).unwrap();
    assert_eq!(rt.tier0(again).unwrap(), rt.tier0(t).unwrap());
    assert_eq!(host.served.len(), before, "what was read stays read");
}

#[test]
fn issue_205_a_window_over_whole_leaves_costs_their_tier0_blocks_and_no_hot_block() {
    let bytes = file();
    let reader = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default()).unwrap();
    let mut host = Host::new(bytes);
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    let opened = host.served.len();
    let mut mem = Runtime::new();
    let m = mem.open(host.bytes.clone()).unwrap();
    let t = host.run(&mut rt, f, |r| r.tier0_viewport(f, everything())).unwrap();
    let a = mem.tier0_viewport(m, everything()).unwrap();
    assert_eq!(rt.tier0(t).unwrap(), mem.tier0(a).unwrap());
    for e in reader.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)) {
        let hit = host.served[opened..]
            .iter()
            .any(|(o, l)| *o < e.offset + u64::from(e.stored_length) && e.offset < o + u64::from(*l));
        assert!(
            !hit,
            "the hot block of cell {} was requested for a window that holds the whole leaf",
            e.cell
        );
    }
}

#[test]
fn issue_205_misuse_is_refused_and_never_panics() {
    let bytes = file();
    let mut host = Host::new(bytes.clone());
    let mut rt = Runtime::new();
    // an unopened handle is not a file
    let h = rt.open_ranged(bytes.len() as u64).unwrap();
    assert_eq!(rt.directory_query(h, everything()), Err(AbiError::WrongKind));
    assert_eq!(rt.query_viewport(h, everything(), 1), Err(AbiError::WrongKind));
    // a range outside the file, or past the end of it
    assert_eq!(rt.provide_range(h, bytes.len() as u64, &[0]), Err(AbiError::BadArgument));
    assert_eq!(rt.provide_range(h, u64::MAX, &[0, 1]), Err(AbiError::BadArgument));
    assert_eq!(rt.open_ranged(0), Err(AbiError::BadArgument));
    // a module with whole-file bytes has no ranges to ask for
    let mut mem = Runtime::new();
    let m = mem.open(bytes.clone()).unwrap();
    assert_eq!(mem.needed(m), Err(AbiError::WrongKind));
    assert_eq!(mem.provide_range(m, 0, &[0]), Err(AbiError::WrongKind));
    assert_eq!(mem.open_poll(m), Err(AbiError::WrongKind));
    // a released handle
    let f = host.open(&mut rt);
    rt.release(f).unwrap();
    assert_eq!(rt.provide_range(f, 0, &[0]), Err(AbiError::Released));
    assert_eq!(rt.needed(f), Err(AbiError::Released));
    // a file with the same length that is not a PVec file
    let junk = rt.open_ranged(bytes.len() as u64).unwrap();
    assert_eq!(rt.open_poll(junk), Err(AbiError::NeedRange));
    for (o, l) in rt.needed(junk).unwrap() {
        rt.provide_range(junk, o, &vec![0x5a; l as usize]).unwrap();
    }
    assert_eq!(rt.open_poll(junk), Err(AbiError::Read));
}

#[test]
fn issue_205_a_host_that_lies_about_a_block_gets_a_read_error() {
    let bytes = file();
    let mut host = Host::new(bytes.clone());
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    // flip bytes in the middle of the file, where blocks are; the blocks must refuse them
    let mid = bytes.len() / 2;
    for b in &mut host.bytes[mid..mid + 4096] {
        *b ^= 0xff;
    }
    let result = host.run(&mut rt, f, |r| r.query_viewport(f, everything(), u32::MAX));
    assert_eq!(result, Err(AbiError::Read));
}
