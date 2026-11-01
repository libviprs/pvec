//! Issue #238: `tier0_leaf`, the Tier 0 stream of one leaf, without the window cull and without a count over the
//! whole file. `docs/WASM_ABI.md` is the contract; every test name carries the issue number.
#![allow(missing_docs)]

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_geometry::{BSpline, Ellipse, Placement2, Point2, Polyline, Vector2};
use pvec_wasm::{AbiError, Runtime, WASM_ABI_VERSION};
use std::fs;
use std::path::Path;

const MAX_ERROR: f64 = 0.05;
const SHORT: u32 = 100;
fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [1000.0, 1000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-238".into(),
        producer_version: "1".into(),
    }
}

fn wave() -> Vec<Point2> {
    (0..300).map(|k| p(f64::from(k) * 3.0, 500.0 + (f64::from(k) * 0.1).sin() * 40.0)).collect()
}

fn file(indexed: Option<(u32, Option<f64>)>) -> Vec<u8> {
    let mut w = match indexed {
        Some((capacity, tier0)) => PvecWriter::new_indexed(manifest(), capacity, tier0).unwrap(),
        None => PvecWriter::new(manifest()).unwrap(),
    };
    for i in 0..SHORT {
        let (x, y) = (f64::from(i % 10) * 10.0, f64::from(i / 10) * 10.0);
        w.push(id(i), &Curve::Polyline(Polyline::new(vec![p(x, y), p(x + 4.0, y + 3.0)]).unwrap())).unwrap();
    }
    w.push(id(100), &Curve::Polyline(Polyline::new(wave()).unwrap())).unwrap();
    let frame = Placement2::new(p(700.0, 700.0), Vector2::new(1.0, 0.0)).unwrap();
    w.push(id(101), &Curve::Ellipse(Ellipse::new(frame, 90.0, 30.0).unwrap())).unwrap();
    for (i, dy) in [(102, 0.0), (103, 5.0)] {
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

fn indexed() -> Vec<u8> {
    file(Some((16, Some(MAX_ERROR))))
}

fn everything() -> Bounds2 {
    Bounds2 { min: [-1_000_000_000.0, -1_000_000_000.0], max: [1_000_000_000.0, 1_000_000_000.0] }
}

fn cells(rt: &mut Runtime, f: u32) -> (Vec<u32>, Vec<u32>) {
    let q = rt.directory_query(f, everything()).unwrap();
    let c = rt.cells(q).unwrap();
    (c.ids.clone(), c.entities.clone())
}

#[test]
fn issue_238_the_abi_stays_2_and_the_document_names_the_call() {
    assert_eq!(WASM_ABI_VERSION, 2);
    let doc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/WASM_ABI.md")).unwrap();
    for needle in ["tier0_leaf", "one leaf", "uncovered"] {
        assert!(doc.contains(needle), "docs/WASM_ABI.md does not mention {needle}");
    }
}

#[test]
fn issue_238_the_leaves_streams_together_are_the_stream_of_everything_and_no_entity_is_in_two() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let all = rt.tier0_viewport(f, everything()).unwrap();
    let all = rt.tier0(all).unwrap().clone();
    let (ids, _) = cells(&mut rt, f);
    assert_eq!(ids.len(), 7);
    let mut seen: Vec<(u32, u32, f64)> = Vec::new();
    for cell in ids {
        let t = rt.tier0_leaf(f, cell).unwrap();
        let s = rt.tier0(t).unwrap();
        assert!(s.ids.windows(2).all(|w| w[0] < w[1]), "cell {cell}: ascending by entity id");
        seen.extend(s.ids.iter().zip(&s.codes).zip(&s.error_bounds).map(|((i, c), e)| (*i, *c, *e)));
    }
    seen.sort_by_key(|r| r.0);
    assert_eq!(seen.len(), all.ids.len(), "each entity once");
    for (row, (id, code, error)) in seen.iter().enumerate() {
        assert_eq!((*id, *code), (all.ids[row], all.codes[row]));
        assert_eq!(error.to_bits(), all.error_bounds[row].to_bits(), "entity {id}");
    }
}

#[test]
fn issue_238_a_leaf_has_the_entries_of_its_own_entities_and_the_previews_are_the_viewports() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let all = rt.tier0_viewport(f, everything()).unwrap();
    let all = rt.tier0(all).unwrap().clone();
    let (ids, entities) = cells(&mut rt, f);
    for (cell, count) in ids.into_iter().zip(entities) {
        let t = rt.tier0_leaf(f, cell).unwrap();
        let s = rt.tier0(t).unwrap();
        assert_eq!(s.ids.len() as u32, count, "cell {cell}: one entry for each hot entity of the leaf");
        for (k, id) in s.ids.iter().enumerate() {
            let at = all.ids.binary_search(id).unwrap();
            let (a, b) = (
                s.offsets[k] as usize * 2..s.offsets[k + 1] as usize * 2,
                all.offsets[at] as usize * 2..all.offsets[at + 1] as usize * 2,
            );
            assert_eq!(s.vertices[a], all.vertices[b], "entity {id}: the same preview vertices");
        }
    }
}

#[test]
fn issue_238_the_counts_are_the_leafs_own() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let (ids, entities) = cells(&mut rt, f);
    for (cell, count) in ids.into_iter().zip(entities) {
        let t = rt.tier0_leaf(f, cell).unwrap();
        let s = rt.tier0(t).unwrap();
        assert_eq!((s.leaves_read, s.leaves_without_block, s.advanced_in_view), (1, 0, 0), "cell {cell}");
        assert_eq!(s.uncovered, 0, "cell {cell}: every entity of a leaf with a block has an entry");
        assert_eq!(s.exact + s.previews + s.infeasible, count);
        assert_eq!(rt.tier0_stat(t, 0), Ok(count));
        assert_eq!(rt.tier0_stat(t, 5), Ok(1));
    }
}

#[test]
fn issue_238_a_leaf_with_no_block_has_no_entries_and_all_its_entities_uncovered() {
    let mut rt = Runtime::new();
    let f = rt.open(file(Some((16, None)))).unwrap();
    let (ids, entities) = cells(&mut rt, f);
    for (cell, count) in ids.into_iter().zip(entities) {
        let t = rt.tier0_leaf(f, cell).unwrap();
        let s = rt.tier0(t).unwrap();
        assert!(s.ids.is_empty());
        assert_eq!((s.leaves_read, s.leaves_without_block, s.uncovered), (0, 1, count), "cell {cell}");
    }
}

#[test]
fn issue_238_misuse_is_refused_and_never_panics() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let (ids, _) = cells(&mut rt, f);
    let beyond = ids.iter().max().unwrap() + 1;
    assert_eq!(rt.tier0_leaf(f, beyond), Err(AbiError::BadArgument));
    assert_eq!(rt.tier0_leaf(f, 0), Err(AbiError::BadArgument), "cell 0 is no leaf");
    assert_eq!(rt.tier0_leaf(f, u32::MAX), Err(AbiError::BadArgument));
    let t = rt.tier0_leaf(f, ids[0]).unwrap();
    assert_eq!(rt.tier0_leaf(t, ids[0]), Err(AbiError::WrongKind));
    let plain = rt.open(file(None)).unwrap();
    assert_eq!(rt.tier0_leaf(plain, 1), Err(AbiError::NoIndex));
    rt.release(f).unwrap();
    assert_eq!(rt.tier0_leaf(f, ids[0]), Err(AbiError::Released));
}

/// The browser: bytes, and a log of what it was asked for.
struct Host {
    bytes: Vec<u8>,
    served: Vec<(u64, u32)>,
}

impl Host {
    fn serve(&mut self, rt: &mut Runtime, handle: u32) {
        for (offset, len) in rt.needed(handle).unwrap() {
            self.served.push((offset, len));
            let end = (offset + u64::from(len)) as usize;
            rt.provide_range(handle, offset, &self.bytes[offset as usize..end]).unwrap();
        }
    }

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
}

#[test]
fn issue_238_a_range_opened_file_reads_only_that_leafs_tier0_block_and_no_hot_block() {
    let bytes = indexed();
    let reader = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default()).unwrap();
    let mut host = Host { bytes: bytes.clone(), served: Vec::new() };
    let mut rt = Runtime::new();
    let h = rt.open_ranged(bytes.len() as u64).unwrap();
    let f = loop {
        match rt.open_poll(h) {
            Ok(f) => break f,
            Err(AbiError::NeedRange) => host.serve(&mut rt, h),
            Err(e) => panic!("open failed: {e:?}"),
        }
    };
    let opened = host.served.len();
    let mut mem = Runtime::new();
    let m = mem.open(bytes).unwrap();
    let (ids, _) = cells(&mut rt, f);
    let cell = ids[3];
    let t = host.run(&mut rt, f, |r| r.tier0_leaf(f, cell)).unwrap();
    let a = mem.tier0_leaf(m, cell).unwrap();
    assert_eq!(rt.tier0(t).unwrap(), mem.tier0(a).unwrap());
    let asked = &host.served[opened..];
    assert!(!asked.is_empty() && asked.len() <= 1, "one block, one range: {asked:?}");
    for e in reader.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)) {
        let hit = asked
            .iter()
            .any(|(o, l)| *o < e.offset + u64::from(e.stored_length) && e.offset < o + u64::from(*l));
        assert!(!hit, "the hot block of cell {} was requested for a Tier 0 leaf", e.cell);
    }
    let before = host.served.len();
    host.run(&mut rt, f, |r| r.tier0_leaf(f, cell)).unwrap();
    assert_eq!(host.served.len(), before, "what was read stays read");
}
