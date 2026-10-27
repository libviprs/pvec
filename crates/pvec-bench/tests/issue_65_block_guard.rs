//! Issue P8.6 (pvec#65): a bigger leaf must not make a block the reader's limits refuse.
//!
//! The index leaf holds many more entities after tuning, and one hot block holds a kind's entities of one
//! leaf. A leaf of long polylines (survey contours are 100 to 1,000 vertices each) can then be tens of
//! megabytes in one block, over the 64 MiB the browser preset allows once it is a few hundred vertices
//! apiece. The container's layout rule allows one hot block of a kind in a cell, so the leaf cannot be cut
//! into two blocks; the pipeline that writes the artifact halves the leaf capacity until every block is
//! under 16 MiB. The reader's limits are not touched: this test opens under a ceiling of exactly 16 MiB.
#![allow(missing_docs)]

use pvec_bench::shape::{Shape, artifact};
use pvec_container::directory::BlockKind;
use pvec_container::source::MemorySource;
use pvec_container::{ContainerReader, LengthUnit, Limits, ReaderConfig};
use pvec_stream::CELL_CAPACITY;

const CEILING: u32 = 16 << 20;

// the default leaf holds all three polylines, so only the byte ceiling can cut them
const _: () = assert!(CELL_CAPACITY >= 3);

/// `n` polylines of 700,000 vertices, about 11 MB each, side by side.
fn contours(n: usize) -> Vec<Shape> {
    (0..n)
        .map(|k| {
            Shape::Polyline(
                (0..700_000).map(|i| [k as f64 * 800.0 + f64::from(i) * 0.001, f64::from(i % 7)]).collect(),
            )
        })
        .collect()
}

fn open(bytes: Vec<u8>, ceiling: u32) -> Result<ContainerReader<MemorySource<Vec<u8>>>, String> {
    let limits = Limits { max_block_bytes: ceiling, ..Limits::DEFAULT };
    ContainerReader::open(MemorySource::new(bytes), ReaderConfig { limits, ..ReaderConfig::default() })
        .map_err(|e| e.to_string())
}

#[test]
fn a_drawing_of_big_polylines_is_written_with_leaves_small_enough_for_a_16_mib_block_ceiling() {
    let shapes = contours(3);
    let bytes = artifact(&shapes, LengthUnit::Metre, "p8.6/block-guard", Default::default(), None).unwrap();
    let r = open(bytes, CEILING).expect("every block is under 16 MiB, so the file opens under that ceiling");
    let hot: Vec<_> = r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).collect();
    assert!(hot.len() >= 2, "{} hot block: 33 MB went into one block", hot.len());
    assert!(hot.iter().all(|e| e.uncompressed_length <= CEILING));
    assert_eq!(hot.iter().map(|e| e.count).sum::<u32>(), 3, "every entity is in exactly one block");
}

#[test]
fn a_drawing_that_fits_keeps_the_default_leaf() {
    let shapes = contours(1);
    let bytes = artifact(&shapes, LengthUnit::Metre, "p8.6/block-guard", Default::default(), None).unwrap();
    let r = open(bytes, CEILING).unwrap();
    assert_eq!(r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).count(), 1);
}
