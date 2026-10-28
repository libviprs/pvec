//! The default size of an advanced chunk (pvec#65 and pvec#192): 256 KiB, the setting at which a viewport that
//! reads only the chunks it touches (`ReadPlan::advanced_chunks`) moves the fewest bytes without the block
//! directory growing past the point where it costs more than it saves (`docs/bench/TUNING.md`).

mod support;

use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::directory::BlockKind;
use pvec_container::source::MemorySource;
use pvec_container::writer::{ADVANCED_CHUNK_BYTES, ContainerWriter, EntityMeta, WriterOptions};
use pvec_container::{Bounds, ContainerReader, ReaderConfig};
use support::*;

#[test]
fn the_default_chunk_is_256_kib() {
    assert_eq!(ADVANCED_CHUNK_BYTES, 256 * 1024);
}

#[test]
fn about_650_kib_of_curves_are_cut_in_three_chunks_by_default_and_none_is_over_the_size() {
    let mut w = ContainerWriter::with_options(config(&[], None), WriterOptions::default());
    for id in 0..2000u32 {
        let x: Vec<f64> = (0..10).map(|j| f64::from(j) + f64::from(id)).collect();
        let y: Vec<f64> = (0..10).map(|j| f64::from((j * 7) % 5)).collect();
        let mut knots = vec![0.0; 4];
        knots.extend((1..=6).map(f64::from));
        knots.extend([7.0; 4]);
        let spec =
            SplineSpec { degree: 3, knots: &knots, x: &x, y: &y, weights: None, closed: false, periodic: false };
        let meta = EntityMeta { id: eid(id), style: 0, cell: 0, bounds: Bounds { min: [0.0, 0.0], max: [10.0, 10.0] } };
        w.push_spline(meta, spec).expect("a spline");
    }
    let bytes = w.finish().expect("the splines write");
    let r = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).expect("opens");
    let chunks = r.advanced_chunk_count().expect("chunks");
    assert!(chunks >= 3, "{chunks} chunks: 2000 splines of 10 control points are over 600 KiB");
    let held: u64 = r
        .entries()
        .iter()
        .filter(|e| matches!(e.block_kind(), Some(BlockKind::Records | BlockKind::Arena)))
        .map(|e| u64::from(e.uncompressed_length))
        .sum();
    assert!(held / chunks as u64 <= u64::from(ADVANCED_CHUNK_BYTES), "{held} bytes in {chunks} chunks");
    assert!(r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Records)).count() == chunks);
}
