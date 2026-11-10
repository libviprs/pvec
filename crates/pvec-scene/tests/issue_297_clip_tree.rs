//! Issue 297: the clip tree of `ClipTree::build` over the golden `blueprint-mix-26-clip.pvec`, and over clips
//! made to fail. A 9-deep chain and an item in two clips are refused by the decoder (COMP-26, COMP-21), so they
//! never reach the crate; the last two tests say so.

use pvec_capabilities::{Reader, Registry, Version, ids};
use pvec_components::{
    Band, ClipRecord, ClipRun, ClipStream, FillRule, ItemKind, MAX_CLIP_DEPTH, Stream, read_streams,
};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_entity::EntityId;
use pvec_scene::{ClipTree, ClippedRun, Counts, Degrade, Policy, ScenePlan};
use std::fs;
use std::path::Path;

fn golden_streams() -> Vec<Stream> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/blueprint-mix-26-clip.pvec");
    let bytes = fs::read(path).expect("the golden file");
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let at = reader
        .entries()
        .iter()
        .position(|e| e.block_kind() == Some(BlockKind::Components))
        .expect("a component block");
    let set = reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec();
    let current = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::CLIP] };
    read_streams(&set, &Registry::v1(), &current).unwrap().streams
}

fn square(_: EntityId) -> Option<Vec<[f64; 2]>> {
    Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
}

fn page() -> Counts {
    Counts { strokes: 8, fills: 3, texts: 1, rasters: vec![Band::Underlay] }
}

fn legacy(counts: &Counts) -> ScenePlan {
    ScenePlan::build(&[], counts, Policy::default())
}

fn stream_of(clips: Vec<ClipRecord>, runs: Vec<ClipRun>) -> Stream {
    ClipStream { clips, runs }.into_stream(&Registry::v1()).unwrap()
}

fn record(parent: Option<u32>, entity: u32) -> ClipRecord {
    ClipRecord { rule: FillRule::NonZero, parent, boundary: vec![EntityId::new(entity).unwrap()] }
}

fn run(kind: ItemKind, first: u32, count: u32, clip: u32) -> ClipRun {
    ClipRun { kind, first, count, clip }
}

#[test]
fn the_golden_file_has_depth_bits_and_masks_that_follow_the_chain() {
    let streams = golden_streams();
    let stream = streams.iter().find(|s| s.capability == ids::CLIP).expect("a clip stream");
    let decoded = ClipStream::decode(&stream.payload).unwrap();
    assert!(!decoded.clips.is_empty());
    let tree = ClipTree::build(&streams, &legacy(&page()), square);
    assert_eq!(tree.degrades, vec![]);
    assert_eq!(tree.nodes.len(), decoded.clips.len());
    for (k, node) in tree.nodes.iter().enumerate() {
        let depth = decoded.chain_of(k as u32).len();
        assert_eq!(usize::from(node.depth), depth, "clip {k}");
        assert_eq!(node.write_bit, 1u8 << (depth - 1), "clip {k}");
        assert_eq!(node.chain_mask, (1u8 << (depth - 1)) - 1, "clip {k}");
        assert_eq!(node.parent, decoded.clips[k].parent);
        assert_eq!(node.rule, decoded.clips[k].rule);
        assert_eq!(node.rings.as_ref().map(Vec::len), Some(decoded.clips[k].boundary.len()));
    }
}

#[test]
fn runs_split_each_batch_at_the_clip_edges_and_cover_every_item_once() {
    let clips = vec![record(None, 1), record(Some(0), 2)];
    let runs = vec![run(ItemKind::Stroke, 2, 2, 0), run(ItemKind::Stroke, 5, 1, 1)];
    let counts = Counts { strokes: 8, fills: 0, texts: 0, rasters: vec![] };
    let plan = legacy(&counts);
    let tree = ClipTree::build(&[stream_of(clips, runs)], &plan, square);
    assert_eq!(plan.batches.len(), 1);
    let run = |first, count, clip| ClippedRun { batch: 0, first, count, clip };
    assert_eq!(
        tree.runs,
        vec![run(0, 2, None), run(2, 2, Some(0)), run(4, 1, None), run(5, 1, Some(1)), run(6, 2, None)]
    );
    assert_eq!(tree.nodes[1].depth, 2);
    assert_eq!((tree.nodes[1].write_bit, tree.nodes[1].chain_mask), (0b10, 0b01));
}

#[test]
fn a_page_without_a_clip_stream_has_one_unclipped_run_per_batch() {
    let plan = legacy(&page());
    let tree = ClipTree::build(&[], &plan, square);
    assert_eq!(tree.nodes, vec![]);
    assert_eq!(tree.degrades, vec![]);
    let want: Vec<ClippedRun> = plan
        .batches
        .iter()
        .enumerate()
        .map(|(batch, b)| ClippedRun { batch, first: b.first, count: b.count, clip: None })
        .collect();
    assert_eq!(tree.runs, want);
}

#[test]
fn a_ring_of_two_points_draws_nothing_and_is_reported_with_its_item_count() {
    let counts = Counts { strokes: 6, fills: 0, texts: 0, rasters: vec![] };
    let streams = [stream_of(vec![record(None, 1)], vec![run(ItemKind::Stroke, 1, 3, 0)])];
    let two = |_: EntityId| Some(vec![[0.0, 0.0], [1.0, 1.0]]);
    let tree = ClipTree::build(&streams, &legacy(&counts), two);
    assert_eq!(tree.degrades, vec![Degrade::ClipUnbuildable { clip: 0, items: 3 }]);
    assert_eq!(tree.nodes[0].rings, None);
    let drawn: u32 = tree.runs.iter().map(|r| r.count).sum();
    assert_eq!(drawn, 3);
    assert!(tree.runs.iter().all(|r| r.clip.is_none()));
}

#[test]
fn a_ring_with_a_nan_draws_nothing() {
    let counts = Counts { strokes: 4, fills: 0, texts: 0, rasters: vec![] };
    let streams = [stream_of(vec![record(None, 1)], vec![run(ItemKind::Stroke, 0, 4, 0)])];
    let nan = |_: EntityId| Some(vec![[0.0, 0.0], [1.0, f64::NAN], [1.0, 1.0]]);
    let tree = ClipTree::build(&streams, &legacy(&counts), nan);
    assert_eq!(tree.degrades, vec![Degrade::ClipUnbuildable { clip: 0, items: 4 }]);
    assert!(tree.runs.is_empty());
}

#[test]
fn a_ring_that_cannot_be_found_is_unbuildable() {
    let counts = Counts { strokes: 2, fills: 0, texts: 0, rasters: vec![] };
    let streams = [stream_of(vec![record(None, 1)], vec![run(ItemKind::Stroke, 0, 2, 0)])];
    let tree = ClipTree::build(&streams, &legacy(&counts), |_| None);
    assert_eq!(tree.degrades, vec![Degrade::ClipUnbuildable { clip: 0, items: 2 }]);
    assert!(tree.runs.is_empty());
}

#[test]
fn a_child_of_an_unbuildable_clip_draws_nothing_and_is_counted_against_the_outermost() {
    let counts = Counts { strokes: 6, fills: 0, texts: 0, rasters: vec![] };
    let clips = vec![record(None, 1), record(Some(0), 2)];
    let runs = vec![run(ItemKind::Stroke, 0, 2, 0), run(ItemKind::Stroke, 2, 2, 1)];
    let bad_root = |e: EntityId| if e.index() == 1 { Some(vec![[0.0, 0.0]]) } else { square(e) };
    let tree = ClipTree::build(&[stream_of(clips, runs)], &legacy(&counts), bad_root);
    assert_eq!(tree.degrades, vec![Degrade::ClipUnbuildable { clip: 0, items: 4 }]);
    assert_eq!(tree.runs, vec![ClippedRun { batch: 0, first: 4, count: 2, clip: None }]);
}

#[test]
fn a_buildable_sibling_is_not_hurt_by_an_unbuildable_one() {
    let counts = Counts { strokes: 4, fills: 0, texts: 0, rasters: vec![] };
    let clips = vec![record(None, 1), record(None, 2)];
    let runs = vec![run(ItemKind::Stroke, 0, 2, 0), run(ItemKind::Stroke, 2, 2, 1)];
    let bad_first = |e: EntityId| if e.index() == 1 { None } else { square(e) };
    let tree = ClipTree::build(&[stream_of(clips, runs)], &legacy(&counts), bad_first);
    assert_eq!(tree.degrades, vec![Degrade::ClipUnbuildable { clip: 0, items: 2 }]);
    assert_eq!(tree.runs, vec![ClippedRun { batch: 0, first: 2, count: 2, clip: Some(1) }]);
}

#[test]
fn a_damaged_clip_stream_is_reported_and_the_items_are_drawn_unclipped() {
    let counts = Counts { strokes: 3, fills: 0, texts: 0, rasters: vec![] };
    let bad = Stream { capability: ids::CLIP, version: Version::new(1, 0, 0), payload: vec![1, 2, 3] };
    let tree = ClipTree::build(&[bad], &legacy(&counts), square);
    assert!(matches!(tree.degrades[..], [Degrade::StreamDamaged { id, .. }] if id == ids::CLIP));
    assert_eq!(tree.runs, vec![ClippedRun { batch: 0, first: 0, count: 3, clip: None }]);
}

#[test]
fn a_chain_nine_deep_cannot_reach_the_crate() {
    let clips: Vec<ClipRecord> =
        (0..=MAX_CLIP_DEPTH as u32).map(|k| record(k.checked_sub(1), k + 1)).collect();
    assert!(ClipStream { clips, runs: vec![] }.into_stream(&Registry::v1()).is_err());
}

#[test]
fn an_item_in_two_clips_cannot_reach_the_crate() {
    let clips = vec![record(None, 1), record(None, 2)];
    let runs = vec![run(ItemKind::Stroke, 0, 2, 0), run(ItemKind::Stroke, 1, 2, 1)];
    assert!(ClipStream { clips, runs }.into_stream(&Registry::v1()).is_err());
}
