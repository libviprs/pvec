//! Issue 298: `sample_gradient` and `sample_tiling` over hand-made gradients and tilings, the golden
//! `blueprint-mix-25-fill-paint.pvec`, and the fill paint payloads and mutants of the fuzz target.

use pvec_capabilities::{Reader, Registry, Version, ids};
use pvec_components::{
    FillPaintStream, FillRule, Gradient, GradientCoords, Stop, Tiling, TilingPrimitive, read_streams,
};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_scene::{
    Degrade, MAX_TILE_COPIES, TILE_POLYGON_EVENODD, TILE_POLYGON_NONZERO, TILE_POLYLINE, sample_gradient,
    sample_tiling,
};
use std::fs;
use std::path::Path;

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const RED: u32 = 0xFF00_00FF;
const GREEN: u32 = 0x00FF_00FF;
const BLUE: u32 = 0x0000_FFFF;
const WHITE: u32 = 0xFFFF_FFFF;

fn words(rgba: u32) -> [f32; 4] {
    [24, 16, 8, 0].map(|s| ((rgba >> s) & 0xFF) as f32 / 255.0)
}

fn gradient(stops: &[(f64, u32)], before: bool, after: bool) -> Gradient {
    Gradient {
        coords: GradientCoords::Axial { x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0 },
        extend_before: before,
        extend_after: after,
        transform: IDENTITY,
        stops: stops.iter().map(|&(offset, rgba)| Stop { offset, rgba }).collect(),
    }
}

/// The texel `n` of a row, where texel 0 is the guard before the range.
fn texel(row: &[f32], n: usize) -> [f32; 4] {
    row[n * 4..n * 4 + 4].try_into().unwrap()
}

fn tiling(cell: [f64; 4], step: [f64; 2], primitives: Vec<TilingPrimitive>) -> Tiling {
    Tiling { coloured: true, x_step: step[0], y_step: step[1], cell, transform: IDENTITY, primitives }
}

fn square(rgba: u32) -> TilingPrimitive {
    TilingPrimitive::Polygon {
        rule: FillRule::NonZero,
        rgba,
        points: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    }
}

#[test]
fn a_two_stop_gradient_samples_exactly_to_its_stops_at_both_ends() {
    let row = sample_gradient(&gradient(&[(0.0, RED), (1.0, BLUE)], true, true), 256);
    assert_eq!(row.len(), (256 + 2) * 4);
    assert_eq!(texel(&row, 1), words(RED));
    assert_eq!(texel(&row, 256), words(BLUE));
    let mid = texel(&row, 1 + 255 / 2);
    assert!(mid[0] > 0.45 && mid[0] < 0.55 && mid[2] > 0.45 && mid[2] < 0.55, "{mid:?}");
}

#[test]
fn an_odd_and_a_tiny_row_still_end_on_the_stops() {
    for n in [2, 3, 17] {
        let row = sample_gradient(&gradient(&[(0.0, GREEN), (1.0, WHITE)], false, false), n);
        assert_eq!(row.len(), (n + 2) * 4);
        assert_eq!(texel(&row, 1), words(GREEN));
        assert_eq!(texel(&row, n), words(WHITE));
    }
    assert_eq!(sample_gradient(&gradient(&[(0.0, GREEN), (1.0, WHITE)], false, false), 0).len(), 4 * 4);
}

#[test]
fn alpha_stays_straight_and_is_not_premultiplied() {
    let row = sample_gradient(&gradient(&[(0.0, 0xFF00_0000), (1.0, 0xFF00_00FF)], true, true), 3);
    assert_eq!(texel(&row, 1), [1.0, 0.0, 0.0, 0.0]);
    let half = texel(&row, 2);
    assert_eq!((half[0], half[3]), (1.0, 0.5 * 1.0));
}

#[test]
fn a_repeated_offset_gives_a_hard_step() {
    let stops = [(0.0, RED), (0.5, GREEN), (0.5, BLUE), (1.0, WHITE)];
    let row = sample_gradient(&gradient(&stops, true, true), 101);
    let (below, at) = (texel(&row, 1 + 49), texel(&row, 1 + 50));
    assert_eq!(at, words(BLUE));
    assert!(below[1] > 0.9 && below[2] < 0.1, "just below the step is still green: {below:?}");
}

#[test]
fn extend_flags_fold_into_a_transparent_texel_outside_the_range() {
    let stops = [(0.25, RED), (0.75, BLUE)];
    let open = sample_gradient(&gradient(&stops, false, false), 5);
    assert_eq!(texel(&open, 0), [0.0; 4]);
    assert_eq!(texel(&open, 1), [0.0; 4], "t = 0 is before the first stop");
    assert_eq!(texel(&open, 3), texel(&open, 3));
    assert_eq!(texel(&open, 5), [0.0; 4], "t = 1 is after the last stop");
    assert_eq!(texel(&open, 6), [0.0; 4]);
    let shut = sample_gradient(&gradient(&stops, true, true), 5);
    assert_eq!(texel(&shut, 0), words(RED));
    assert_eq!(texel(&shut, 1), words(RED));
    assert_eq!(texel(&shut, 5), words(BLUE));
    assert_eq!(texel(&shut, 6), words(BLUE));
    let one_side = sample_gradient(&gradient(&stops, true, false), 5);
    assert_eq!((texel(&one_side, 0), texel(&one_side, 6)), (words(RED), [0.0; 4]));
}

#[test]
fn a_gradient_with_no_stops_is_transparent_and_does_not_panic() {
    let row = sample_gradient(&gradient(&[], true, true), 4);
    assert!(row.iter().all(|v| *v == 0.0) && row.len() == 6 * 4);
}

#[test]
fn a_cell_inside_its_step_is_one_copy() {
    let t = tiling([0.0, 0.0, 1.0, 1.0], [1.0, 1.0], vec![square(RED)]);
    let s = sample_tiling(&t).unwrap();
    assert_eq!(s.copies, [1, 1]);
    assert_eq!((s.kinds.clone(), s.colours.clone(), s.widths.clone()), (vec![TILE_POLYGON_NONZERO], vec![RED], vec![0.0]));
    assert_eq!(s.starts, vec![0, 4]);
    assert_eq!(s.points, vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
}

#[test]
fn a_cell_box_over_its_step_gives_ceil_of_the_ratio_copies_per_axis() {
    let prims = vec![square(RED), square(BLUE)];
    for (cell, step, copies) in [
        ([0.0, 0.0, 3.0, 2.0], [2.0, 2.0], [2, 1]),
        ([0.0, 0.0, 2.0, 2.0], [2.0, 2.0], [1, 1]),
        ([0.0, 0.0, 2.1, 4.5], [2.0, 2.0], [2, 3]),
        ([-1.0, -1.0, 4.0, 1.0], [1.0, 2.0], [5, 1]),
    ] {
        let s = sample_tiling(&tiling(cell, step, prims.clone())).unwrap();
        assert_eq!(s.copies, copies, "{cell:?} over {step:?}");
        let n = (copies[0] * copies[1]) as usize * prims.len();
        assert_eq!((s.kinds.len(), s.colours.len(), s.widths.len(), s.starts.len()), (n, n, n, n + 1));
        assert_eq!(s.points.len(), n * 4 * 2);
        assert_eq!(s.starts[n], (n * 4) as u32);
    }
}

#[test]
fn neighbour_copies_come_first_shifted_back_by_whole_steps_and_the_cell_itself_last() {
    let t = tiling([0.0, 0.0, 3.0, 1.0], [2.0, 1.0], vec![square(RED)]);
    let s = sample_tiling(&t).unwrap();
    assert_eq!(s.copies, [2, 1]);
    assert_eq!(s.points[0], -2.0, "the copy one step to the left first");
    assert_eq!(s.points[8], 0.0, "the cell itself last");
}

#[test]
fn polygon_rules_and_polylines_keep_their_kind_colour_and_width() {
    let t = tiling(
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 1.0],
        vec![
            TilingPrimitive::Polygon { rule: FillRule::EvenOdd, rgba: GREEN, points: vec![[0.0; 2], [1.0, 0.0], [0.0, 1.0]] },
            TilingPrimitive::Polyline { rgba: 0x1234_5678, width: 0.25, points: vec![[0.0, 0.0], [1.0, 1.0]] },
        ],
    );
    let s = sample_tiling(&t).unwrap();
    assert_eq!(s.kinds, vec![TILE_POLYGON_EVENODD, TILE_POLYLINE]);
    assert_eq!(s.colours, vec![GREEN, 0x1234_5678]);
    assert_eq!(s.widths, vec![0.0, 0.25]);
    assert_eq!(s.starts, vec![0, 3, 5]);
}

#[test]
fn a_tiling_past_the_copy_cap_is_reported_and_one_at_the_cap_is_expanded() {
    let at_cap = tiling([0.0, 0.0, 8.0, 8.0], [1.0, 1.0], vec![square(RED)]);
    assert_eq!(sample_tiling(&at_cap).unwrap().copies, [8, 8]);
    assert_eq!(MAX_TILE_COPIES, 64);
    let past = tiling([0.0, 0.0, 9.0, 8.0], [1.0, 1.0], vec![square(RED)]);
    assert_eq!(sample_tiling(&past), Err(Degrade::TilingTooLarge));
    let absurd = tiling([0.0, 0.0, 1e300, 1e300], [1e-300, 1e-300], vec![square(RED)]);
    assert_eq!(sample_tiling(&absurd), Err(Degrade::TilingTooLarge));
}

fn all_finite(g: &[Gradient], t: &[Tiling]) -> usize {
    let mut seen = 0;
    for g in g {
        for n in [2, 256] {
            let row = sample_gradient(g, n);
            assert_eq!(row.len(), (n + 2) * 4);
            assert!(row.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{g:?}");
            seen += 1;
        }
    }
    for t in t {
        if let Ok(s) = sample_tiling(t) {
            assert!(s.points.iter().chain(&s.widths).all(|v| v.is_finite()), "{t:?}");
            assert_eq!(*s.starts.last().unwrap() as usize * 2, s.points.len());
            seen += 1;
        }
    }
    seen
}

#[test]
fn every_number_is_finite_on_the_golden_file() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/blueprint-mix-25-fill-paint.pvec");
    let reader =
        ContainerReader::open(MemorySource::new(fs::read(path).unwrap()), ReaderConfig::default()).unwrap();
    let at = reader.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Components)).unwrap();
    let set = reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec();
    let current = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::FILL_PAINT] };
    let streams = read_streams(&set, &Registry::v1(), &current).unwrap().streams;
    let stream = streams.iter().find(|s| s.capability == FillPaintStream::CAPABILITY).expect("stream 25");
    let paint = FillPaintStream::decode(&stream.payload).unwrap();
    assert!(!paint.gradients.is_empty() && !paint.tilings.is_empty());
    assert_eq!(all_finite(&paint.gradients, &paint.tilings), paint.gradients.len() * 2 + paint.tilings.len());
}

#[test]
fn every_number_is_finite_on_the_fuzz_corpus_and_on_extreme_values() {
    let mut seen = 0;
    for input in pvec_fuzz::seeds::for_target("fuzz_fill_paint", &[]) {
        if let Ok(p) = FillPaintStream::decode(&input) {
            seen += all_finite(&p.gradients, &p.tilings);
        }
    }
    assert!(seen > 0, "the corpus reached no decoded payload");
    let big = f64::MAX;
    let t = tiling(
        [-big, -big, big, big],
        [big, big],
        vec![TilingPrimitive::Polyline { rgba: RED, width: big, points: vec![[big, -big], [-big, big]] }],
    );
    assert_eq!(all_finite(&[], &[t]), 1);
    let near = tiling([0.0, 0.0, 1.5 * big, 1.0], [big, 1.0], vec![square(RED)]);
    assert_eq!(all_finite(&[], &[near]), 1);
    let g = gradient(&[(f64::NAN, RED), (f64::INFINITY, BLUE)], true, true);
    assert_eq!(all_finite(&[g], &[]), 2);
}
