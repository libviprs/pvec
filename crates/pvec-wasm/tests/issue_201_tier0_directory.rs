//! Issue #201: the Tier 0 stream (P5.4) and the spatial directory (P5.1) through the WASM ABI, version 2.
//! `docs/WASM_ABI.md` is the contract; every test name carries the issue number.

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter, Transform2};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_geometry::{BSpline, Ellipse, Placement2, Point2, Polyline, Vector2};
use pvec_wasm::{
    AbiError, Precision, Runtime, TIER0_CHORD_POLYLINE, TIER0_EXACT, WASM_ABI_VERSION,
};
use std::fs;
use std::path::Path;

const MAX_ERROR: f64 = 0.05;
const SHORT: u32 = 100;
/// 100 short segments, the wave (100), the ellipse (101), two splines (102, 103).
const ENTITIES: u32 = 104;
const HOT: u32 = 102;

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
        producer_id: "issue-201".into(),
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

fn window(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds2 {
    Bounds2 { min: [x0, y0], max: [x1, y1] }
}

#[test]
fn issue_201_the_abi_is_version_2_and_the_document_names_the_new_exports() {
    assert_eq!(WASM_ABI_VERSION, 2);
    let doc = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/WASM_ABI.md")).unwrap();
    for needle in [
        "**2**",
        "directory_stat",
        "directory_bound",
        "directory_query",
        "tier0_viewport",
        "tier0_stat",
        "tier0_buffer_ptr",
        "-6",
        "version 1",
    ] {
        assert!(doc.contains(needle), "docs/WASM_ABI.md does not mention {needle}");
    }
}

#[test]
fn issue_201_the_version_1_exports_still_work_on_an_indexed_file() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let batch = rt.query_viewport(f, everything(), u32::MAX).unwrap();
    assert_eq!(rt.batch_len(batch).unwrap(), ENTITIES);
    let ctx = pvec::TessellationContext {
        source_to_map: Transform2::IDENTITY,
        map_to_screen: Transform2::scale(8.0, 8.0).unwrap(),
        pixel_ratio: 1.0,
        max_screen_error_px: 0.25,
        max_world_error: f64::INFINITY,
        max_segments: 4096,
    };
    let t = rt.tessellate_batch(batch, &ctx, Precision::F64).unwrap();
    assert_eq!(rt.entity_ids(t).unwrap().len(), ENTITIES as usize);
    assert_eq!(AbiError::Released.code(), -1);
    assert_eq!(AbiError::WrongKind.code(), -5);
}

#[test]
fn issue_201_the_directory_reports_cells_height_entities_and_bounds() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let info = rt.directory(f).unwrap().expect("a directory");
    assert_eq!((info.cells, info.entities, info.tier0_leaves), (7, HOT, 7));
    assert!(info.height >= 1);
    assert!(info.bounds.min[0] <= 0.0 && info.bounds.max[0] >= 897.0 && info.bounds.max[1] >= 790.0);
    assert!(info.advanced_bounds.is_some());
}

#[test]
fn issue_201_a_file_without_a_directory_says_so_and_every_index_call_is_no_index() {
    let mut rt = Runtime::new();
    let f = rt.open(file(None)).unwrap();
    assert!(rt.directory(f).unwrap().is_none());
    assert_eq!(rt.directory_query(f, everything()), Err(AbiError::NoIndex));
    assert_eq!(rt.tier0_viewport(f, everything()), Err(AbiError::NoIndex));
    assert_eq!(AbiError::NoIndex.code(), -6);
}

#[test]
fn issue_201_a_directory_query_lists_the_cells_a_window_touches_in_ascending_order() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let all = rt.directory_query(f, everything()).unwrap();
    let cells = rt.cells(all).unwrap().clone();
    assert_eq!(cells.ids.len(), 7);
    assert!(cells.ids.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(cells.entities.iter().sum::<u32>(), HOT);
    assert_eq!(cells.has_tier0, vec![1; 7]);
    assert_eq!(cells.bounds.len(), 7 * 4);
    assert!(cells.stored_bytes.iter().all(|b| *b > 0.0));
    let small = rt.directory_query(f, window(0.0, 0.0, 5.0, 5.0)).unwrap();
    let few = rt.cells(small).unwrap();
    assert!(!few.ids.is_empty() && few.ids.len() < 7);
    assert!(few.ids.iter().all(|c| cells.ids.contains(c)));
    let far = rt.directory_query(f, window(5_000_000.0, 5_000_000.0, 6_000_000.0, 6_000_000.0)).unwrap();
    assert!(rt.cells(far).unwrap().ids.is_empty());
}

#[test]
fn issue_201_tier0_over_everything_covers_each_hot_entity_once_and_counts_the_rest_as_uncovered() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let t = rt.tier0_viewport(f, everything()).unwrap();
    let s = rt.tier0(t).unwrap();
    assert_eq!(s.ids.len(), HOT as usize);
    assert_eq!(s.ids, (0..HOT).collect::<Vec<u32>>());
    assert_eq!((s.exact, s.previews, s.infeasible), (100, 2, 0));
    assert_eq!(s.uncovered, 2, "the two splines are in no leaf");
    assert_eq!(s.leaves_read, 7);
    assert_eq!(s.leaves_without_block, 0);
    assert_eq!(s.advanced_in_view, 1);
    assert_eq!(s.codes.len(), s.ids.len());
    assert_eq!(s.error_bounds.len(), s.ids.len());
    assert_eq!(s.offsets.len(), s.ids.len() + 1);
}

#[test]
fn issue_201_a_preview_declares_a_finite_error_and_an_exact_entry_has_none() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let t = rt.tier0_viewport(f, everything()).unwrap();
    let s = rt.tier0(t).unwrap();
    for (i, entity) in s.ids.iter().enumerate() {
        let range = s.offsets[i] as usize..s.offsets[i + 1] as usize;
        if s.codes[i] == TIER0_EXACT {
            assert_eq!(s.error_bounds[i], 0.0, "entity {entity}");
            assert!(range.is_empty(), "an exact entry carries no vertices: its hot block has them");
        } else {
            assert!(s.error_bounds[i] > 0.0 && s.error_bounds[i] <= MAX_ERROR, "entity {entity}");
            assert!(!range.is_empty(), "entity {entity}");
        }
    }
    assert_eq!(s.codes[100], TIER0_CHORD_POLYLINE, "the wave");
}

#[test]
fn issue_201_the_declared_error_holds_against_the_exact_vertices() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let t = rt.tier0_viewport(f, everything()).unwrap();
    let s = rt.tier0(t).unwrap();
    let preview: Vec<[f64; 2]> =
        s.vertices[2 * s.offsets[100] as usize..2 * s.offsets[101] as usize].chunks(2).map(|c| [c[0], c[1]]).collect();
    let dist = |q: Point2| -> f64 {
        preview
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dy * dy;
                let u = if len2 == 0.0 { 0.0 } else { (((q.x - a[0]) * dx + (q.y - a[1]) * dy) / len2).clamp(0.0, 1.0) };
                ((a[0] + u * dx - q.x).powi(2) + (a[1] + u * dy - q.y).powi(2)).sqrt()
            })
            .fold(f64::INFINITY, f64::min)
    };
    let worst = wave().into_iter().map(dist).fold(0.0, f64::max);
    assert!(worst <= s.error_bounds[100], "worst {worst} against declared {}", s.error_bounds[100]);
}

#[test]
fn issue_201_a_window_gets_exactly_the_entities_the_viewport_query_gets_less_the_uncovered() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    for w in [
        window(0.0, 0.0, 25.0, 25.0),
        window(0.0, 480.0, 400.0, 560.0),
        window(650.0, 650.0, 800.0, 800.0),
        window(0.0, 880.0, 200.0, 1000.0),
        everything(),
    ] {
        let batch = rt.query_viewport(f, w, u32::MAX).unwrap();
        let wanted: Vec<u32> = {
            let t = rt.tessellate_batch(batch, &small_ctx(), Precision::F64).unwrap();
            rt.entity_ids(t).unwrap().to_vec()
        };
        let t0 = rt.tier0_viewport(f, w).unwrap();
        let s = rt.tier0(t0).unwrap();
        let hot: Vec<u32> = wanted.iter().copied().filter(|e| *e < HOT).collect();
        assert_eq!(s.ids, hot, "{w:?}");
        assert_eq!(s.uncovered as usize, wanted.len() - hot.len(), "{w:?}");
    }
}

fn small_ctx() -> pvec::TessellationContext {
    pvec::TessellationContext {
        source_to_map: Transform2::IDENTITY,
        map_to_screen: Transform2::IDENTITY,
        pixel_ratio: 1.0,
        max_screen_error_px: 1.0,
        max_world_error: f64::INFINITY,
        max_segments: 4096,
    }
}

#[test]
fn issue_201_a_directory_without_tier0_blocks_is_all_uncovered_and_says_which_leaves_lacked_one() {
    let mut rt = Runtime::new();
    let f = rt.open(file(Some((16, None)))).unwrap();
    let t = rt.tier0_viewport(f, everything()).unwrap();
    let s = rt.tier0(t).unwrap();
    assert!(s.ids.is_empty());
    assert_eq!(s.uncovered, ENTITIES);
    assert_eq!((s.leaves_read, s.leaves_without_block), (0, 7));
}

#[test]
fn issue_201_a_damaged_directory_opens_but_the_index_calls_are_read_errors() {
    let mut bytes = indexed();
    let at = {
        let r = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default()).unwrap();
        let e = r.entries().iter().find(|e| e.kind == 3).expect("a directory block");
        e.offset as usize + e.stored_length as usize / 2
    };
    bytes[at] ^= 0xFF;
    let mut rt = Runtime::new();
    let f = rt.open(bytes).expect("the entities do not depend on the directory");
    assert_eq!(rt.directory(f), Err(AbiError::Read));
    assert_eq!(rt.tier0_viewport(f, everything()), Err(AbiError::Read));
    assert_eq!(AbiError::Read.code(), -4);
}

#[test]
fn issue_201_a_result_is_five_arrays_whatever_the_number_of_entries() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    for w in [window(0.0, 0.0, 5.0, 5.0), everything()] {
        let t = rt.tier0_viewport(f, w).unwrap();
        let views: Vec<_> = (0..5).map(|k| rt.tier0_view(t, k).unwrap()).collect();
        assert_eq!(views.len(), 5);
        assert_eq!(rt.tier0_view(t, 5), Err(AbiError::BadArgument));
    }
    let c = rt.directory_query(f, everything()).unwrap();
    assert_eq!((0..5).filter(|k| rt.cells_view(c, *k).is_ok()).count(), 5);
    assert_eq!(rt.cells_view(c, 5), Err(AbiError::BadArgument));
}

#[test]
fn issue_201_the_new_handles_obey_the_handle_rules() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let t = rt.tier0_viewport(f, everything()).unwrap();
    let c = rt.directory_query(f, everything()).unwrap();
    assert_eq!(rt.tier0(f).err(), Some(AbiError::WrongKind));
    assert_eq!(rt.tier0(c).err(), Some(AbiError::WrongKind));
    assert_eq!(rt.cells(t).err(), Some(AbiError::WrongKind));
    assert_eq!(rt.tier0_viewport(t, everything()), Err(AbiError::WrongKind));
    assert_eq!(rt.directory(t).err(), Some(AbiError::WrongKind));
    rt.release(t).unwrap();
    assert_eq!(rt.tier0(t).err(), Some(AbiError::Released));
    assert_eq!(rt.release(t), Err(AbiError::Released));
    let nan = Bounds2 { min: [f64::NAN, 0.0], max: [1.0, 1.0] };
    assert_eq!(rt.tier0_viewport(f, nan), Err(AbiError::BadArgument));
    assert_eq!(rt.directory_query(f, nan), Err(AbiError::BadArgument));
}

#[test]
fn issue_201_a_result_survives_releasing_the_file_and_the_same_query_gives_the_same_arrays() {
    let mut rt = Runtime::new();
    let f = rt.open(indexed()).unwrap();
    let a = rt.tier0_viewport(f, everything()).unwrap();
    let b = rt.tier0_viewport(f, everything()).unwrap();
    rt.release(f).unwrap();
    assert_eq!(rt.tier0(a).unwrap(), rt.tier0(b).unwrap());
}

#[test]
fn issue_201_the_bindings_export_the_index_calls_as_numbers() {
    let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bindings.rs")).unwrap();
    for name in [
        "directory_stat",
        "directory_bound",
        "directory_query",
        "cells_len",
        "cells_ptr",
        "cells_elem",
        "tier0_viewport",
        "tier0_stat",
        "tier0_buffer_len",
        "tier0_buffer_ptr",
        "tier0_buffer_elem",
    ] {
        assert!(source.contains(&format!("pub fn {name}(")), "the export `{name}` is missing");
    }
}
