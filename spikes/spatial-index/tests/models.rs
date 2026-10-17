//! The models must find every visible entity, the curves must be curves, and the behaviours the
//! comparison rests on (root congestion, bounded duplication) must actually show up.

use pvec_fixtures::{Drawing, generate, viewport_script};
use spatial_index_spike::buckets::Buckets;
use spatial_index_spike::cells::Cells;
use spatial_index_spike::curve::{hilbert, morton};
use spatial_index_spike::quadtree::Quadtree;
use spatial_index_spike::rtree::RTree;
use spatial_index_spike::*;

fn models(items: &Items) -> Vec<Box<dyn Model>> {
    vec![
        Box::new(Quadtree::build(items, 10)),
        Box::new(RTree::build(items, 512, 64)),
        Box::new(RTree::build(items, 64, 32)),
        Box::new(Cells::build(items, 10, false)),
        Box::new(Cells::build(items, 10, true)),
        Box::new(Buckets::build(items, 7)),
        Box::new(Buckets::build(items, 8)),
    ]
}

fn rects(d: &Drawing) -> Vec<[f64; 4]> {
    let mut v: Vec<[f64; 4]> = viewport_script().iter().map(|s| d.viewport(s)).collect();
    // Plus rectangles that poke outside the world, hug an edge, or are a single point.
    let [x0, y0, x1, y1] = d.extent;
    let (w, h) = (x1 - x0, y1 - y0);
    v.push([x0 - w, y0 - h, x0 + w * 0.01, y0 + h * 0.01]);
    v.push([x1 - w * 0.01, y1 - h * 0.01, x1 + w, y1 + h]);
    v.push([x0 + w * 0.5, y0 + h * 0.5, x0 + w * 0.5, y0 + h * 0.5]);
    v.push([x0 + w * 0.3, y0 - h * 2.0, x0 + w * 0.4, y0 - h]);
    v
}

fn check_complete(d: &Drawing) {
    let items = items_of(d);
    for m in models(&items) {
        for r in rects(d) {
            let plan = m.query(&items, &r);
            let mut got = plan.fetched.clone();
            got.sort_unstable();
            got.dedup();
            for e in truth(&items, &r) {
                assert!(got.binary_search(&e).is_ok(), "{} missed entity {e} for {r:?}", m.name());
            }
        }
    }
}

#[test]
fn every_model_finds_every_visible_entity_on_the_small_fixture() {
    check_complete(&generate::small_synthetic());
}

#[test]
fn every_model_finds_every_visible_entity_with_full_extent_entities_present() {
    let mut d = generate::small_synthetic();
    // Add the four full-extent entities by hand rather than generating the 100k fixture in a debug test.
    let w = d.extent[2];
    for g in [
        pvec_fixtures::Geom::Line { a: [0.0, w / 2.0], b: [w, w / 2.0] },
        pvec_fixtures::Geom::Line { a: [0.0, 0.0], b: [w, w] },
        pvec_fixtures::Geom::Circle { c: [w / 2.0, w / 2.0], r: w / 2.0 },
    ] {
        d.entities.push(pvec_fixtures::Entity { id: d.entities.len() as u32, geom: g });
    }
    check_complete(&d);
}

#[test]
fn a_whole_drawing_query_returns_every_entity_exactly_once_for_single_ownership_models() {
    let d = generate::small_synthetic();
    let items = items_of(&d);
    let all = [
        items.world[0] - 1.0,
        items.world[1] - 1.0,
        items.world[0] + items.world[2] + 1.0,
        items.world[1] + items.world[2] + 1.0,
    ];
    for m in [
        Box::new(Quadtree::build(&items, 10)) as Box<dyn Model>,
        Box::new(RTree::build(&items, 64, 8)),
        Box::new(Buckets::build(&items, 7)),
    ] {
        let mut got = m.query(&items, &all).fetched;
        got.sort_unstable();
        assert_eq!(got, (0..items.len() as u32).collect::<Vec<_>>(), "{}", m.name());
        assert_eq!(m.duplication(), 1.0);
    }
}

#[test]
fn duplication_in_the_reference_model_is_bounded_by_four() {
    let d = generate::small_synthetic();
    let items = items_of(&d);
    let m = Cells::build(&items, 10, false);
    assert!(m.duplication() >= 1.0 && m.duplication() <= 4.0, "duplication {}", m.duplication());
}

#[test]
fn the_size_model_matches_layout_b_per_kind_to_within_eight_percent() {
    // Layout B's measured bytes per entity on `million` (docs/spikes/01-5-layouts.md), kind by kind.
    // The straddler fixture has the same mix and generator with a different seed.
    let want = [(0usize, 36.7), (1, 28.7), (2, 44.7), (3, 60.7), (4, 128.7), (5, 237.9), (6, 212.3)];
    let d = generate::straddler();
    let mut sum = [0f64; 7];
    let mut n = [0f64; 7];
    for e in &d.entities {
        sum[e.geom.tag() as usize] += size_model(&e.geom) as f64;
        n[e.geom.tag() as usize] += 1.0;
    }
    for (k, b) in want {
        let model = sum[k] / n[k];
        assert!((model - b).abs() / b < 0.08, "kind {k}: model {model:.1} against layout B {b}");
    }
}

#[test]
fn congestion_of_long_entities_shows_in_the_quadtree_root_and_not_in_the_others() {
    let d = with_long_polylines(&generate::small_synthetic(), 200, 200, 7);
    let items = items_of(&d);
    let long_bytes: u64 = (1000..1200).map(|i| items.size[i] as u64).sum();
    assert!(long_bytes > 600_000, "200 polylines of 200 vertices are {long_bytes} bytes");
    let q = Quadtree::build(&items, 10);
    let root = q.worst(&items)[0].1;
    assert!(root >= long_bytes as f64, "root holds {root}");
    // A tiny viewport far from every long polyline still pays for the root under ancestor ownership.
    let corner = [d.extent[0], d.extent[1], d.extent[0] + 10.0, d.extent[1] + 10.0];
    let plan = q.query(&items, &corner);
    let (_, bytes) = merge(&plan.all_reads(), 0);
    assert!(bytes >= long_bytes, "the quadtree fetched {bytes} for a 10 x 10 viewport");
    // The same viewport under the bucket model reads the small chunk table and nothing of the long payload.
    let b = Buckets::build(&items, 7);
    let (_, bytes_b) = merge(&b.query(&items, &corner).all_reads(), 0);
    assert!(bytes_b < long_bytes / 10, "buckets fetched {bytes_b}");
}

#[test]
fn hilbert_visits_every_cell_once_and_steps_to_a_neighbour() {
    let order = 4;
    let n = 1u32 << order;
    let mut by_d = vec![(0u32, 0u32); (n * n) as usize];
    let mut seen = vec![false; (n * n) as usize];
    for x in 0..n {
        for y in 0..n {
            let d = hilbert(order, x, y) as usize;
            assert!(!seen[d], "index {d} reached twice");
            seen[d] = true;
            by_d[d] = (x, y);
        }
    }
    for w in by_d.windows(2) {
        let (a, b) = (w[0], w[1]);
        assert_eq!(a.0.abs_diff(b.0) + a.1.abs_diff(b.1), 1, "{a:?} to {b:?} is not a unit step");
    }
}

#[test]
fn morton_interleaves_bits() {
    assert_eq!(morton(0b1, 0b0), 0b01);
    assert_eq!(morton(0b0, 0b1), 0b10);
    assert_eq!(morton(0b11, 0b11), 0b1111);
    assert_eq!(morton(0b101, 0b010), 0b011001);
}

#[test]
fn merging_handles_overlap_adjacency_and_gaps() {
    let r = [(100, 10), (110, 10), (125, 5), (105, 10), (1000, 1)];
    assert_eq!(merge(&r, 0), (3, 26));
    assert_eq!(merge(&r, 5), (2, 31));
    assert_eq!(merge(&[], 5), (0, 0));
}
