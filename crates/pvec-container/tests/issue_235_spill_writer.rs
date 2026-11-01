//! Issue #235: the writer that keeps its pushes in files writes the artifact the in-memory writer writes, byte
//! for byte, for the whole fixture set, every codec and integrity option, every budget, and every order of
//! pushes; it refuses what the in-memory writer refuses, with the same rule; and a sink that fails, or a
//! build that refuses, leaves nothing behind and writes nothing.

mod support;

use pvec_components::{RasterId, Stream};
use pvec_container::Bounds;
use pvec_container::blocks::raster::TileIn;
use pvec_container::compress::{Codec, CodecPolicy};
use pvec_container::integrity::Integrity;
use pvec_container::spill_writer::{FinishError, SpillWriter};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError, WriterOptions};
use std::io;
use std::path::{Path, PathBuf};
use support::raster::{placement, tiles, two_placements};
use support::*;

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pvec235-container-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn left(dir: &Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

fn options() -> Vec<WriterOptions> {
    let mut out = vec![WriterOptions::default()];
    for integrity in [Integrity::None, Integrity::Checksums] {
        for codec in [Codec::None, Codec::Lz4] {
            out.push(WriterOptions { integrity, codecs: CodecPolicy::uniform(codec) });
        }
    }
    out
}

#[test]
fn every_fixture_streams_to_the_bytes_the_in_memory_writer_writes_at_every_budget_and_option() {
    let tmp = root("fixtures");
    for fx in fixtures() {
        for seed in [0u64, 1] {
            let input = if seed == 0 { fx.clone() } else { fx.shuffled(seed) };
            for opt in options() {
                let expect = input.write_with(opt).unwrap();
                for budget in [1usize, 20_000, 1 << 30] {
                    let got = input.write_spilled(&tmp, budget, opt).unwrap();
                    assert!(
                        got == expect,
                        "{} seed {seed} {opt:?} budget {budget}: the bytes differ",
                        fx.name
                    );
                }
            }
        }
    }
    assert_eq!(left(&tmp), 0);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn a_sink_that_fails_at_any_byte_is_an_output_error_and_leaves_no_file() {
    let tmp = root("sink");
    let fx = mixed();
    let total = fx.write().unwrap().len();
    let mut room = 0;
    while room < total {
        let mut sink = FullSink { room, taken: 0 };
        let err =
            fx.load_spill(&tmp, 500, WriterOptions::default()).unwrap().finish_to(&mut sink).unwrap_err();
        assert!(
            matches!(err, FinishError::Output(ref e) if e.source_ref().downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::StorageFull)),
            "{err:?}"
        );
        assert_eq!(left(&tmp), 0);
        room += 1 + total / 37;
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn a_build_that_refuses_writes_nothing_and_gives_the_rule_the_in_memory_writer_gives() {
    let tmp = root("rules");
    let base = minimal();
    // each case: a name, the input, and the rule the in-memory writer refuses it with
    let mut cases: Vec<(&str, Input)> = Vec::new();
    let mut twice = base.clone();
    twice.items.push(base.items[0].clone());
    cases.push(("an id used twice", twice));
    let mut unknown = base.clone();
    unknown.config = config(&[0xdead], None);
    cases.push(("an unknown capability", unknown));
    let mut bounds = base.clone();
    bounds.config = config(&[], Some(Bounds { min: [f64::NAN, 0.0], max: [1.0, 1.0] }));
    cases.push(("bad source bounds", bounds));
    let mut streams = base.clone();
    let s = Stream {
        capability: pvec_capabilities::ids::LAYER,
        version: pvec_capabilities::Version::new(1, 0, 0),
        payload: vec![0; 8],
    };
    streams.items.push(Item::Stream(s.clone()));
    streams.items.push(Item::Stream(s));
    cases.push(("two streams of one capability", streams));
    let mut empty = base.clone();
    empty.items.clear();
    cases.push(("nothing to write", empty));
    let mut onspline = base.clone();
    onspline.items.push(Item::Ent(Ent::Composite {
        id: 9,
        style: 0,
        segments: vec![0],
        trims: None,
        closed: false,
    }));
    cases.push(("a composite is fine", onspline));
    for (name, input) in cases {
        let expect = input.write();
        let mut sink = VecSink::default();
        let got = match input.load_spill(&tmp, 1 << 20, WriterOptions::default()) {
            Ok(w) => w.finish_to(&mut sink),
            Err(e) => Err(e),
        };
        match (expect, got) {
            (Ok(bytes), Ok(n)) => {
                assert_eq!(n as usize, sink.0.len(), "{name}");
                assert!(bytes == sink.0, "{name}");
            }
            (Err(a), Err(FinishError::Write(b))) => {
                assert_eq!(a.rule(), b.rule(), "{name}: {a} against {b}");
                assert!(sink.0.is_empty(), "{name}: the sink got bytes");
            }
            (a, b) => panic!("{name}: in memory {a:?}, spilled {b:?}"),
        }
        assert_eq!(left(&tmp), 0, "{name}");
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn a_push_the_container_refuses_is_refused_at_the_push_with_the_same_rule() {
    let tmp = root("push");
    let meta = |cell, bounds| EntityMeta { id: eid(1), style: 0, cell, bounds };
    let ok = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
    let bad = Bounds { min: [2.0, 0.0], max: [1.0, 1.0] };
    let mut m = ContainerWriter::new(config(&[], None));
    let mut s = SpillWriter::new(config(&[], None), store(&tmp), 1024).unwrap();
    let a = m.push_line(meta(0, bad), [0.0; 2], [1.0; 2]).unwrap_err();
    let b = s.push_line(meta(0, bad), [0.0; 2], [1.0; 2]).unwrap_err();
    assert!(matches!(b, FinishError::Write(ref e) if e.rule() == a.rule()), "{a} against {b}");
    let knots = [0.0, 0.0, 1.0, 1.0];
    let spec = || pvec_container::blocks::advanced::SplineSpec {
        degree: 1,
        knots: &knots,
        x: &[0.0, 1.0],
        y: &[0.0, 1.0],
        weights: None,
        closed: false,
        periodic: false,
    };
    let a = m.push_spline(meta(3, ok), spec()).unwrap_err();
    let b = s.push_spline(meta(3, ok), spec()).unwrap_err();
    assert!(matches!(a, WriteError::CellOnAdvanced { .. }));
    assert!(matches!(b, FinishError::Write(WriteError::CellOnAdvanced { .. })), "{b:?}");
    drop(s);
    assert_eq!(left(&tmp), 0);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn rasters_stream_to_the_bytes_the_in_memory_writer_writes_whatever_the_chunking() {
    let tmp = root("rasters");
    let ps = two_placements();
    let stream = || Stream {
        capability: pvec_components::RASTER_CAPABILITY,
        version: pvec_capabilities::Version::new(1, 0, 0),
        payload: pvec_components::encode_rasters(&ps).unwrap(),
    };
    let line = EntityMeta { id: eid(0), style: 0, cell: 0, bounds: Bounds { min: [0.0; 2], max: [10.0; 2] } };
    for (chunk, inline) in
        [(1u32 << 26, 256u64 * 1024), (1500, 256 * 1024), (3000, 0), (700, 1000), (4096, 10_000_000)]
    {
        let batches: Vec<Vec<TileIn>> = vec![
            tiles(0, &ps[0], 700, |l, c, r| l == 2 && (c + r) % 2 == 1),
            tiles(1, &ps[1], 700, |_, _, _| false),
        ];
        let mut m = ContainerWriter::new(config(&[], None));
        m.push_line(line, [0.0; 2], [10.0; 2]).unwrap();
        m.push_stream(stream()).unwrap();
        m.raster_chunk_bytes(chunk);
        m.raster_inline_bytes(inline);
        for (r, t) in batches.iter().enumerate() {
            m.push_raster_tiles(RasterId::new(r as u32).unwrap(), t.clone()).unwrap();
        }
        let expect = m.finish().unwrap();
        for budget in [1usize, 1 << 20] {
            let mut s = SpillWriter::new(config(&[], None), store(&tmp), budget).unwrap();
            s.push_line(line, [0.0; 2], [10.0; 2]).unwrap();
            s.push_stream(&stream()).unwrap();
            s.raster_chunk_bytes(chunk);
            s.raster_inline_bytes(inline);
            // the second raster first, and each in two batches: the arrival order must not show
            for r in [1usize, 0] {
                let (a, b) = batches[r].split_at(batches[r].len() / 2);
                s.push_raster_tiles(RasterId::new(r as u32).unwrap(), b.to_vec()).unwrap();
                s.push_raster_tiles(RasterId::new(r as u32).unwrap(), a.to_vec()).unwrap();
            }
            let mut out = VecSink::default();
            s.finish_to(&mut out).unwrap();
            assert!(out.0 == expect, "chunk {chunk} inline {inline} budget {budget}");
        }
    }
    // a tile the sniffer refuses, and tiles for a raster nobody placed
    let mut s = SpillWriter::new(config(&[], None), store(&tmp), 1024).unwrap();
    s.push_line(line, [0.0; 2], [10.0; 2]).unwrap();
    s.push_stream(&stream()).unwrap();
    s.push_raster_tiles(
        RasterId::new(0).unwrap(),
        vec![TileIn { level: 0, col: 0, row: 0, bytes: vec![1, 2, 3] }],
    )
    .unwrap();
    let mut sink = VecSink::default();
    let err = s.finish_to(&mut sink).unwrap_err();
    assert!(matches!(err, FinishError::Write(WriteError::Tile { .. })), "{err:?}");
    assert!(sink.0.is_empty());
    let mut s = SpillWriter::new(config(&[], None), store(&tmp), 1024).unwrap();
    s.push_line(line, [0.0; 2], [10.0; 2]).unwrap();
    s.push_stream(&stream()).unwrap();
    let p = placement(10, 10, 256);
    let _ = p;
    s.push_raster_tiles(RasterId::new(7).unwrap(), tiles(7, &ps[1], 700, |_, _, _| false)).unwrap();
    let err = s.finish_to(&mut VecSink::default()).unwrap_err();
    assert!(matches!(err, FinishError::Write(WriteError::Raster(_))), "{err:?}");
    assert_eq!(left(&tmp), 0);
    std::fs::remove_dir_all(&tmp).unwrap();
}
