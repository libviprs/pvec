//! The raster plan against a real file (issue pvec#220): the placement, the index block and the tile blocks are
//! read from an artifact the test-only tiler's pyramid was written into, and the plan is held to the bytes.

use pvec_components::{Band, PixelFormat, Placement, TileCodec};
use pvec_container::blocks::raster::KIND_TILES;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::{ContainerReader, ReaderConfig};
use pvec_stream::{Aabb, RasterPlanError, TileSlot, ViewportQuery, plan_placed};
use pvec_testkit::raster_file::{RasterIn, line_writer, raster_file};
use pvec_testkit::tiler::{Shape, tiles};

/// The test vector of docs/RASTER.md chapter 5, cut at 256: 1024 by 512 pixels, three levels.
const SHAPE: Shape = Shape { width: 1024, height: 512, tile_edge: 256, channels: 1, bit_depth: 8 };

fn placement(codec: TileCodec) -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
        width: 1024,
        height: 512,
        visible_box: [0, 0, 1024, 512],
        tile_edge: 256,
        levels: 3,
        tile_codec: codec,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }
}

/// One raster: level 2 (full resolution) has its tile (3, 1) not stored, level 0 is inline.
fn file() -> Vec<u8> {
    let tiles = tiles(SHAPE, |level, col, row| !(level == 2 && (col, row) == (3, 1)));
    raster_file(
        line_writer(),
        &[RasterIn { placement: placement(TileCodec::Png), tiles, inline_levels: 1, chunk_bytes: 1 << 22 }],
        1 << 20,
    )
}

fn open(bytes: &[u8]) -> ContainerReader<Recording<MemorySource<Vec<u8>>>> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes.to_vec())), ReaderConfig::default())
        .expect("opens")
}

fn whole_footprint(scale: f64) -> ViewportQuery {
    // the footprint of the vector: x 100 to 740, y -206 to 178
    ViewportQuery::new(Aabb::new([100.0, -206.0], [740.0, 178.0]).unwrap(), scale)
}

#[test]
fn the_planned_ranges_are_the_bytes_the_reader_returns_for_each_tile() {
    let bytes = file();
    let reader = open(&bytes);
    let index = reader.raster_index(0).unwrap();
    let plan = plan_placed(
        &reader,
        0,
        &placement(TileCodec::Png),
        &index,
        &whole_footprint(1.0),
        &ReaderConfig::default(),
    )
    .unwrap();
    assert_eq!(plan.level, 2, "one device pixel per source unit is full resolution here");
    let mut stored = 0;
    for t in &plan.tiles {
        let TileSlot::Stored { offset, len, .. } = t.slot else { continue };
        stored += 1;
        let read = reader.read_raster_tile(0, plan.level, t.col, t.row).unwrap().expect("a stored tile");
        assert_eq!(
            &bytes[offset as usize..offset as usize + len as usize],
            read.as_slice(),
            "tile {:?}",
            (t.col, t.row)
        );
        assert!(plan.ranges.iter().any(|&(o, l)| o <= offset && offset + u64::from(len) <= o + u64::from(l)));
    }
    assert_eq!(stored, 7, "eight tiles of the level, one of them not stored");
    assert!(!plan.tiles.iter().any(|t| (t.col, t.row) == (3, 1)), "the not-stored tile is not planned");
}

#[test]
fn no_planned_range_touches_a_geometry_block_or_an_index_block() {
    let bytes = file();
    let reader = open(&bytes);
    let index = reader.raster_index(0).unwrap();
    let plan = plan_placed(
        &reader,
        0,
        &placement(TileCodec::Png),
        &index,
        &whole_footprint(1.0),
        &ReaderConfig::default(),
    )
    .unwrap();
    assert!(!plan.ranges.is_empty());
    for &(offset, len) in &plan.ranges {
        let end = offset + u64::from(len);
        for e in reader.entries() {
            let (s, t) = (e.offset, e.offset + u64::from(e.stored_length));
            if s < end && offset < t {
                assert_eq!(
                    e.kind, KIND_TILES,
                    "a range of {offset}..{end} overlaps a block of kind {}",
                    e.kind
                );
            }
        }
    }
}

#[test]
fn an_inline_tile_is_present_and_the_coarse_level_costs_no_request() {
    let bytes = file();
    let reader = open(&bytes);
    let index = reader.raster_index(0).unwrap();
    let mut config = ReaderConfig::default();
    config.limits.max_plan_decoded_bytes = 256 * 256; // one tile: forces the coarsest level
    let plan =
        plan_placed(&reader, 0, &placement(TileCodec::Png), &index, &whole_footprint(1.0), &config).unwrap();
    assert_eq!(plan.level, 0);
    assert_eq!(plan.tiles.len(), 1);
    assert_eq!(plan.tiles[0].slot, TileSlot::Inline);
    assert!(plan.ranges.is_empty());
}

#[test]
fn limit_16_comes_from_the_reader_configuration() {
    let bytes = file();
    let reader = open(&bytes);
    let index = reader.raster_index(0).unwrap();
    let mut config = ReaderConfig::default();
    config.limits.max_plan_decoded_bytes = 100;
    let r = plan_placed(&reader, 0, &placement(TileCodec::Png), &index, &whole_footprint(1.0), &config);
    assert_eq!(r, Err(RasterPlanError::OverBudget));
}

#[test]
fn a_raster_with_an_unknown_codec_is_skipped_and_not_planned() {
    let bytes = file();
    let reader = open(&bytes);
    let index = reader.raster_index(0).unwrap();
    let r = plan_placed(
        &reader,
        0,
        &placement(TileCodec::Unknown(9)),
        &index,
        &whole_footprint(1.0),
        &ReaderConfig::default(),
    );
    assert_eq!(r, Err(RasterPlanError::Skipped));
}

#[test]
fn planning_reads_nothing_after_the_index() {
    let bytes = file();
    let reader = open(&bytes);
    let index = reader.raster_index(0).unwrap();
    let before = reader.source().requests().len();
    plan_placed(
        &reader,
        0,
        &placement(TileCodec::Png),
        &index,
        &whole_footprint(1.0),
        &ReaderConfig::default(),
    )
    .unwrap();
    assert_eq!(reader.source().requests().len(), before);
}
