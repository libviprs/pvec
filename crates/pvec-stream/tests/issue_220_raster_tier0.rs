//! The raster share of Tier 0 and the reported extent (issue pvec#220). A file is a directory over one line,
//! rasters written after it by the test-only raster file, and `plan_tiered` over it.
#![allow(missing_docs)]

use pvec_components::{Band, PixelFormat, Placement, TileCodec};
use pvec_container::blocks::raster::{KIND_INDEX, KIND_TILES};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, EntityBox, RASTER_TIER0_CAP, SpatialDirectory, Tier, Tier0Leaves, ViewportConfig, ViewportQuery,
    build_directory, reported_extent,
};
use pvec_testkit::raster_file::{RasterIn, raster_file};
use pvec_testkit::tiler::{Shape, tiles};

const SHAPE: Shape = Shape { width: 1024, height: 512, tile_edge: 256, channels: 1, bit_depth: 8 };

fn placement() -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
        width: 1024,
        height: 512,
        visible_box: [0, 0, 1024, 512],
        tile_edge: 256,
        levels: 3,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }
}

fn raster() -> RasterIn {
    RasterIn {
        placement: placement(),
        tiles: tiles(SHAPE, |_, _, _| true),
        inline_levels: 1,
        chunk_bytes: 1 << 22,
    }
}

fn bounds(a: Aabb) -> Bounds {
    Bounds { min: a.min(), max: a.max() }
}

/// One line with a spatial directory over it, and `rasters` after the geometry.
fn writer() -> (ContainerWriter, Aabb) {
    let line = Aabb::new([0.0, 0.0], [1.0, 1.0]).unwrap();
    let built = build_directory(&[EntityBox { id: EntityId::new(0).unwrap(), bounds: line }], 64).unwrap();
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata { producer_id: "t".into(), producer_version: "0".into(), corpus_revision: None },
        bounds: None,
        capabilities: Vec::new(),
    });
    let meta = EntityMeta {
        id: EntityId::new(0).unwrap(),
        style: 0,
        cell: built.cells[0].get(),
        bounds: bounds(line),
    };
    w.push_line(meta, [0.0, 0.0], [1.0, 1.0]).unwrap();
    w.set_spatial_directory(built.nodes, bounds(built.bounds), built.bytes.clone()).unwrap();
    (w, built.bounds)
}

type Mem = Recording<MemorySource<Vec<u8>>>;

fn open(bytes: Vec<u8>) -> (ContainerReader<Mem>, SpatialDirectory, Tier0Leaves) {
    let r = ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap();
    let dir = SpatialDirectory::open(&r).unwrap().expect("a directory");
    let leaves = Tier0Leaves::new(r.entries(), &dir).unwrap();
    (r, dir, leaves)
}

/// The viewport sees the vector and the whole footprint of the raster (x 100 to 740, y -206 to 178).
fn view() -> ViewportQuery {
    ViewportQuery::new(Aabb::new([-10.0, -300.0], [800.0, 300.0]).unwrap(), 1.0)
}

fn index_blocks(r: &ContainerReader<Mem>) -> Vec<BlockId> {
    (0..r.entries().len() as u32)
        .map(BlockId)
        .filter(|b| r.entries()[b.0 as usize].kind == KIND_INDEX)
        .collect()
}

#[test]
fn a_file_with_no_raster_plans_exactly_as_it_did_before_rasters() {
    let (w, _) = writer();
    let (r, dir, leaves) = open(w.finish().unwrap());
    let q = view();
    let before = q.plan(&dir, &r, &ViewportConfig::default()).unwrap();
    for cap in [0, RASTER_TIER0_CAP, u64::MAX] {
        let cfg = ViewportConfig { raster_tier0_cap: cap, ..ViewportConfig::default() };
        assert_eq!(q.plan_tiered(&dir, &r, &leaves, &cfg).unwrap(), before, "cap {cap}");
    }
}

#[test]
fn the_index_blocks_of_the_rasters_in_view_are_planned_at_tier_0_and_no_tile_block_is() {
    let (w, _) = writer();
    let (r, dir, leaves) = open(raster_file(w, &[raster(), raster()], 1 << 20));
    let cfg = ViewportConfig { raster_tier0_cap: u64::MAX, ..ViewportConfig::default() };
    let plan = view().plan_tiered(&dir, &r, &leaves, &cfg).unwrap();
    let tier0: Vec<BlockId> = plan
        .requests
        .iter()
        .chain(&plan.deferred)
        .filter(|p| p.tier == Tier::Tier0)
        .flat_map(|p| p.request.blocks.iter().copied())
        .filter(|b| r.entries()[b.0 as usize].kind == KIND_INDEX)
        .collect();
    assert_eq!(tier0, index_blocks(&r), "both index blocks, in raster order");
    assert!(plan.block_ids().iter().all(|b| r.entries()[b.0 as usize].kind != KIND_TILES));
    let bytes: u64 =
        index_blocks(&r).iter().map(|b| u64::from(r.entries()[b.0 as usize].stored_length)).sum();
    assert!(
        plan.metrics.tier0.bytes_requested >= bytes,
        "the raster bytes are counted against the Tier 0 class"
    );
}

#[test]
fn the_cap_stops_the_raster_share_and_the_budget_defers_it() {
    let (w, _) = writer();
    let (r, dir, leaves) = open(raster_file(w, &[raster(), raster()], 1 << 20));
    let one = u64::from(r.entries()[index_blocks(&r)[0].0 as usize].stored_length);
    let planned = |cfg: ViewportConfig, budget: u64| {
        let mut q = view();
        q.tier_budget = budget;
        q.plan_tiered(&dir, &r, &leaves, &cfg).unwrap()
    };
    let count = |plan: &pvec_stream::ReadPlan| {
        plan.block_ids().iter().filter(|b| r.entries()[b.0 as usize].kind == KIND_INDEX).count()
    };
    let capped = ViewportConfig { raster_tier0_cap: one + one / 2, ..ViewportConfig::default() };
    assert_eq!(count(&planned(capped, u64::MAX)), 1, "a cap of one and a half index blocks admits one");
    let zero = ViewportConfig { raster_tier0_cap: 0, ..ViewportConfig::default() };
    assert_eq!(count(&planned(zero, u64::MAX)), 0);
    // the always-granted first request goes, the rest of a tight budget is deferred
    let wide = ViewportConfig { raster_tier0_cap: u64::MAX, ..ViewportConfig::default() };
    let tight = planned(wide, 0);
    assert_eq!(tight.requests.len(), 1);
    assert!(
        !tight.deferred.is_empty() || count(&tight) == 2,
        "a request past the budget is deferred, or the two index blocks are one request"
    );
}

#[test]
fn the_reported_extent_includes_the_raster_footprint_and_the_root_box_does_not() {
    let (w, root) = writer();
    let (r, dir, _) = open(raster_file(w, &[raster()], 1 << 20));
    assert_eq!(dir.bounds(), root, "the directory's root box is a union of entities");
    let extent = reported_extent(dir.bounds(), &[placement()]);
    // the footprint of RASTER.md chapter 5: x 100 to 740, y -206 to 178
    assert_eq!((extent.min(), extent.max()), ([0.0, -206.0], [740.0, 178.0]));
    assert!(!dir.bounds().contains(&extent));
    assert_eq!(reported_extent(dir.bounds(), &[]), dir.bounds());
    let _ = r;
}
