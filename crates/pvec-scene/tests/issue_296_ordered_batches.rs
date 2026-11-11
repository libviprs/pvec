//! Issue 296: the plan of `ScenePlan::build` over the golden `blueprint-mix-23-paint-order.pvec` (eight
//! entities, three fills, one raster, one text record), and over streams made to fail.

use pvec_capabilities::{Reader, Registry, Version, ids};
use pvec_components::{Band, ItemKind, PaintOrderStream, PaintRun, Stream, read_streams};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_scene::{Counts, DEFAULT_MAX_BATCHES, Degrade, PaintBatch, Policy, ScenePlan};
use std::fs;
use std::path::Path;

fn golden_streams() -> Vec<Stream> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/blueprint-mix-23-paint-order.pvec");
    let bytes = fs::read(path).expect("the golden file");
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let at = reader
        .entries()
        .iter()
        .position(|e| e.block_kind() == Some(BlockKind::Components))
        .expect("a component block");
    let set = reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec();
    let current = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::PAINT_ORDER] };
    read_streams(&set, &Registry::v1(), &current).unwrap().streams
}

fn page() -> Counts {
    Counts { strokes: 8, fills: 3, texts: 1, rasters: vec![Band::Underlay] }
}

fn batch(kind: ItemKind, first: u32, count: u32) -> PaintBatch {
    PaintBatch { kind, first, count }
}

fn stream_of(runs: Vec<PaintRun>) -> Stream {
    PaintOrderStream { runs }.into_stream(&Registry::v1()).unwrap()
}

fn legacy_of(counts: &Counts) -> Vec<PaintBatch> {
    ScenePlan::build(&[], counts, Policy::default()).batches
}

#[test]
fn the_golden_file_gives_the_listed_order_after_the_unlisted_strokes() {
    let plan = ScenePlan::build(&golden_streams(), &page(), Policy::default());
    assert_eq!(plan.degrades, vec![]);
    assert_eq!(
        plan.batches,
        vec![
            batch(ItemKind::Stroke, 4, 4),
            batch(ItemKind::Fill, 0, 1),
            batch(ItemKind::Raster, 0, 1),
            batch(ItemKind::Fill, 1, 2),
            batch(ItemKind::Stroke, 0, 4),
            batch(ItemKind::Text, 0, 1),
        ]
    );
}

#[test]
fn a_page_with_no_stream_gets_the_legacy_stage_order() {
    let counts = Counts {
        strokes: 3,
        fills: 2,
        texts: 1,
        rasters: vec![Band::Overlay, Band::Underlay, Band::Underlay],
    };
    let plan = ScenePlan::build(&[], &counts, Policy::default());
    assert_eq!(plan.degrades, vec![]);
    assert_eq!(
        plan.batches,
        vec![
            batch(ItemKind::Raster, 1, 2),
            batch(ItemKind::Fill, 0, 2),
            batch(ItemKind::Stroke, 0, 3),
            batch(ItemKind::Text, 0, 1),
            batch(ItemKind::Raster, 0, 1),
        ]
    );
}

#[test]
fn an_empty_page_has_no_batches() {
    let counts = Counts { strokes: 0, fills: 0, texts: 0, rasters: vec![] };
    assert_eq!(
        ScenePlan::build(&[], &counts, Policy::default()),
        ScenePlan { batches: vec![], degrades: vec![] }
    );
}

#[test]
fn adjacent_runs_of_one_kind_join_into_one_batch() {
    let run = |first, count| PaintRun { kind: ItemKind::Stroke, first, count };
    let counts = Counts { strokes: 6, fills: 0, texts: 0, rasters: vec![] };
    let plan = ScenePlan::build(&[stream_of(vec![run(0, 2), run(2, 4)])], &counts, Policy::default());
    assert_eq!(plan.batches, vec![batch(ItemKind::Stroke, 0, 6)]);
}

#[test]
fn unlisted_items_around_a_listed_range_come_first_in_legacy_order() {
    let counts = Counts { strokes: 6, fills: 0, texts: 2, rasters: vec![] };
    let listed = vec![PaintRun { kind: ItemKind::Stroke, first: 2, count: 2 }];
    let plan = ScenePlan::build(&[stream_of(listed)], &counts, Policy::default());
    assert_eq!(
        plan.batches,
        vec![
            batch(ItemKind::Stroke, 0, 2),
            batch(ItemKind::Stroke, 4, 2),
            batch(ItemKind::Text, 0, 2),
            batch(ItemKind::Stroke, 2, 2),
        ]
    );
}

#[test]
fn a_run_over_max_batches_returns_the_legacy_plan_and_one_entry() {
    let streams = golden_streams();
    let plan = ScenePlan::build(&streams, &page(), Policy { max_batches: 5 });
    assert_eq!(plan.degrades, vec![Degrade::TooManyBatches]);
    assert_eq!(plan.batches, legacy_of(&page()));
    // exactly at the cap is fine
    let at_cap = ScenePlan::build(&streams, &page(), Policy { max_batches: 6 });
    assert_eq!(at_cap.degrades, vec![]);
    assert_eq!(at_cap.batches.len(), 6);
}

#[test]
fn an_alternating_page_over_the_default_cap_degrades() {
    let n = DEFAULT_MAX_BATCHES as u32;
    let mut runs = Vec::new();
    for i in 0..n {
        runs.push(PaintRun { kind: ItemKind::Stroke, first: i, count: 1 });
        runs.push(PaintRun { kind: ItemKind::Fill, first: i, count: 1 });
    }
    let counts = Counts { strokes: n, fills: n, texts: 0, rasters: vec![] };
    let plan = ScenePlan::build(&[stream_of(runs)], &counts, Policy::default());
    assert_eq!(plan.degrades, vec![Degrade::TooManyBatches]);
    assert_eq!(plan.batches, vec![batch(ItemKind::Fill, 0, n), batch(ItemKind::Stroke, 0, n)]);
}

#[test]
fn a_corrupted_stream_23_is_reported_and_the_legacy_plan_is_used() {
    let mut streams = golden_streams();
    // a kind tag the format does not define, in the first run
    streams[0].payload[4..8].copy_from_slice(&9u32.to_le_bytes());
    let plan = ScenePlan::build(&streams, &page(), Policy::default());
    assert_eq!(plan.degrades, vec![Degrade::StreamDamaged { id: ids::PAINT_ORDER, rule: "COMP-11" }]);
    assert_eq!(plan.batches, legacy_of(&page()));
}

#[test]
fn a_truncated_or_empty_payload_never_panics() {
    let good = golden_streams().remove(0);
    for len in 0..good.payload.len() {
        let mut s = good.clone();
        s.payload.truncate(len);
        let plan = ScenePlan::build(&[s], &page(), Policy::default());
        assert_eq!(plan.degrades.len(), 1, "length {len}");
        assert_eq!(plan.batches, legacy_of(&page()), "length {len}");
    }
}

#[test]
fn a_stream_that_names_an_item_the_page_lacks_is_damaged() {
    let counts = Counts { strokes: 8, fills: 3, texts: 0, rasters: vec![Band::Underlay] };
    let plan = ScenePlan::build(&golden_streams(), &counts, Policy::default());
    assert_eq!(plan.degrades, vec![Degrade::StreamDamaged { id: ids::PAINT_ORDER, rule: "COMP-22" }]);
    assert_eq!(plan.batches, legacy_of(&counts));
}

#[test]
fn streams_of_other_capabilities_are_ignored() {
    let mut streams = golden_streams();
    streams.insert(
        0,
        Stream { capability: ids::STROKE_PAINT, version: Version::new(1, 0, 0), payload: vec![1, 2, 3] },
    );
    let plan = ScenePlan::build(&streams, &page(), Policy::default());
    assert_eq!(plan.degrades, vec![]);
    assert_eq!(plan.batches.len(), 6);
}
