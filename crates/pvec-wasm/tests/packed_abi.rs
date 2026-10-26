//! Issue #54: packed, versioned buffer interfaces for the browser. Each test name carries the issue
//! number; `docs/WASM_ABI.md` is the contract they hold the implementation to.

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter, Transform2};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_wasm::{AbiError, BufferKind, DISPLAY_FIDELITY, ElemType, Precision, Runtime, WASM_ABI_VERSION};
use std::fs;
use std::path::Path;

const BIG: f64 = 1_000_000_000.0;

fn context(max_segments: usize) -> pvec::TessellationContext {
    pvec::TessellationContext {
        source_to_map: Transform2::IDENTITY,
        map_to_screen: Transform2::scale(8.0, 8.0).unwrap(),
        pixel_ratio: 1.0,
        max_screen_error_px: 0.25,
        max_world_error: f64::INFINITY,
        max_segments,
    }
}

fn everything() -> Bounds2 {
    Bounds2 { min: [-BIG, -BIG], max: [BIG, BIG] }
}

/// `n` short segments on a grid, entity `i + 1` at column `i % 1000`, row `i / 1000`, with style `i % 7`.
fn grid(n: u32) -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "issue-54".into(),
            producer_version: "1".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    for i in 0..n {
        let (x, y) = (f64::from(i % 1000), f64::from(i / 1000));
        let (a, b) = ([x, y], [x + 0.5, y + 0.25]);
        let meta = EntityMeta {
            id: EntityId::new(i + 1).unwrap(),
            style: i % 7,
            cell: 0,
            bounds: Bounds { min: a, max: b },
        };
        w.push_line(meta, a, b).unwrap();
    }
    w.finish().unwrap()
}

fn packed(rt: &mut Runtime, n: u32, bounds: Bounds2, budget: u32) -> (u32, u32, u32) {
    let file = rt.open(grid(n)).unwrap();
    let batch = rt.query_viewport(file, bounds, budget).unwrap();
    let t = rt.tessellate_batch(batch, &context(64), Precision::F64).unwrap();
    (file, batch, t)
}

#[test]
fn issue_54_the_abi_version_is_one_and_the_document_names_every_array() {
    assert_eq!(WASM_ABI_VERSION, 1);
    let doc = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/WASM_ABI.md"))
        .expect("docs/WASM_ABI.md exists");
    for needle in [
        "wasm_abi_version",
        "Float64Array",
        "Float32Array",
        "Uint32Array",
        "stride",
        "offset table",
        "memory.grow",
        "release",
        "DisplayFidelity",
    ] {
        assert!(doc.contains(needle), "docs/WASM_ABI.md does not mention {needle}");
    }
}

#[test]
fn issue_54_a_viewport_query_makes_at_most_four_js_visible_allocations_whatever_n_is() {
    for n in [1, 1_000, 100_000] {
        let mut rt = Runtime::new();
        let (_, batch, t) = packed(&mut rt, n, everything(), u32::MAX);
        assert_eq!(rt.batch_len(batch).unwrap(), n);
        let views = rt.views(t).unwrap();
        assert!(views.len() <= 4, "N = {n}");
        // the arrays grow with N, the count of them does not
        assert_eq!(rt.entity_ids(t).unwrap().len(), n as usize);
        assert_eq!(views[BufferKind::EntityIds as usize].len, n);
    }
}

#[test]
fn issue_54_the_arrays_agree_with_the_offset_table() {
    let mut rt = Runtime::new();
    let (_, _, t) = packed(&mut rt, 50, everything(), u32::MAX);
    let offsets = rt.offsets(t).unwrap();
    let vertices = rt.vertices_f64(t).unwrap();
    let ids = rt.entity_ids(t).unwrap();
    assert_eq!(offsets.len(), ids.len() + 1);
    assert_eq!(offsets[0], 0);
    assert!(offsets.windows(2).all(|w| w[0] < w[1]), "a segment is at least two vertices");
    assert_eq!(*offsets.last().unwrap() as usize * 2, vertices.len(), "x and y interleaved");
    assert_eq!(ids, (1..=50).collect::<Vec<u32>>());
    let styles = rt.style_refs(t).unwrap();
    assert_eq!(styles, (0..50).map(|i| i % 7).collect::<Vec<u32>>());
    // entity 0 is the segment (0, 0) to (0.5, 0.25)
    assert_eq!(&vertices[..4], &[0.0, 0.0, 0.5, 0.25]);
    let v = rt.views(t).unwrap();
    assert_eq!(v[BufferKind::Vertices as usize].elem, ElemType::F64);
    assert_eq!(v[BufferKind::Vertices as usize].len as usize, vertices.len());
    assert_eq!(v[BufferKind::Offsets as usize].elem, ElemType::U32);
    assert_eq!(v[BufferKind::Offsets as usize].len as usize, offsets.len());
}

#[test]
fn issue_54_a_viewport_keeps_what_it_meets_and_a_budget_says_when_it_cut() {
    let mut rt = Runtime::new();
    let file = rt.open(grid(3000)).unwrap();
    // columns 0..=9 of rows 0..=2
    let window = Bounds2 { min: [-0.5, -0.5], max: [9.9, 2.9] };
    let batch = rt.query_viewport(file, window, u32::MAX).unwrap();
    assert_eq!(rt.batch_len(batch).unwrap(), 30);
    assert!(!rt.batch_truncated(batch).unwrap());
    let cut = rt.query_viewport(file, window, 12).unwrap();
    assert_eq!(rt.batch_len(cut).unwrap(), 12);
    assert!(rt.batch_truncated(cut).unwrap());
}

#[test]
fn issue_54_the_float32_form_is_relative_to_an_origin_the_batch_reports() {
    let mut rt = Runtime::new();
    let file = rt.open(grid(10)).unwrap();
    let batch = rt.query_viewport(file, everything(), u32::MAX).unwrap();
    let t = rt.tessellate_batch(batch, &context(64), Precision::F32).unwrap();
    let v = rt.view(t, BufferKind::Vertices).unwrap();
    assert_eq!(v.elem, ElemType::F32);
    let origin = rt.origin(t).unwrap();
    let small = rt.vertices_f32(t).unwrap().to_vec();
    assert_eq!(small.len(), v.len as usize);
    let exact = {
        let t64 = rt.tessellate_batch(batch, &context(64), Precision::F64).unwrap();
        rt.vertices_f64(t64).unwrap().to_vec()
    };
    for (i, s) in small.iter().enumerate() {
        let want = exact[i] - origin[i % 2];
        assert_eq!(*s, want as f32, "vertex component {i}");
    }
    assert!(matches!(rt.vertices_f64(t), Err(AbiError::WrongKind)));
}

#[test]
fn issue_54_the_conformance_tag_reaches_the_consumer() {
    let mut rt = Runtime::new();
    let (_, _, t) = packed(&mut rt, 5, everything(), u32::MAX);
    assert_eq!(rt.conformance(t).unwrap(), DISPLAY_FIDELITY);
    assert_eq!(DISPLAY_FIDELITY, 1);
}

#[test]
fn issue_54_an_entity_that_misses_its_budget_gets_no_vertices_and_is_counted() {
    let mut rt = Runtime::new();
    let mut w = PvecWriter::new(Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-54".into(),
        producer_version: "1".into(),
    })
    .unwrap();
    let p = |x: f64, y: f64| pvec_geometry::Point2 { x, y };
    let circle = pvec_geometry::Circle::new(
        pvec_geometry::Placement2::new(p(50.0, 50.0), pvec_geometry::Vector2::new(1.0, 0.0)).unwrap(),
        40.0,
    )
    .unwrap();
    w.push(EntityId::new(1).unwrap(), &Curve::Circle(circle)).unwrap();
    w.push(
        EntityId::new(2).unwrap(),
        &Curve::Polyline(pvec_geometry::Polyline::new(vec![p(0.0, 0.0), p(1.0, 1.0)]).unwrap()),
    )
    .unwrap();
    let file = rt.open(w.finish().unwrap()).unwrap();
    let batch = rt.query_viewport(file, everything(), u32::MAX).unwrap();
    let t = rt.tessellate_batch(batch, &context(4), Precision::F64).unwrap();
    assert_eq!(rt.failed(t).unwrap(), 1, "the circle cannot be held to 0.25 px in 4 segments");
    let offsets = rt.offsets(t).unwrap();
    assert_eq!(offsets, &[0, 0, 2]);
    assert_eq!(rt.entity_ids(t).unwrap(), &[1, 2]);
}

#[test]
fn issue_54_release_is_safe_twice_and_a_released_handle_is_never_read() {
    let mut rt = Runtime::new();
    let (file, batch, t) = packed(&mut rt, 10, everything(), u32::MAX);
    assert_eq!(rt.release(t), Ok(()));
    assert_eq!(rt.release(t), Err(AbiError::Released));
    assert_eq!(rt.views(t), Err(AbiError::Released));
    assert_eq!(rt.offsets(t), Err(AbiError::Released));
    assert_eq!(rt.view_current(t, rt.memory_epoch()), Err(AbiError::Released));
    // a batch outlives its file: it holds what it needs
    assert_eq!(rt.release(file), Ok(()));
    assert_eq!(rt.query_viewport(file, everything(), 1), Err(AbiError::Released));
    assert_eq!(rt.batch_len(batch), Ok(10));
    assert_eq!(rt.release(batch), Ok(()));
    assert_eq!(rt.batch_len(batch), Err(AbiError::Released));
    // a handle that was never issued
    assert_eq!(rt.release(0xffff_ffff), Err(AbiError::Released));
    // a handle of the wrong kind is refused, not misread
    let (file2, _, t2) = packed(&mut rt, 2, everything(), u32::MAX);
    assert_eq!(rt.views(file2), Err(AbiError::WrongKind));
    assert_eq!(rt.tessellate_batch(t2, &context(8), Precision::F64), Err(AbiError::WrongKind));
}

#[test]
fn issue_54_a_reused_slot_does_not_revive_an_old_handle() {
    let mut rt = Runtime::new();
    let first = rt.open(grid(2)).unwrap();
    rt.release(first).unwrap();
    let second = rt.open(grid(2)).unwrap();
    assert_ne!(first, second, "the generation is part of the handle");
    assert_eq!(rt.query_viewport(first, everything(), 1), Err(AbiError::Released));
    assert!(rt.query_viewport(second, everything(), 1).is_ok());
}

#[test]
fn issue_54_after_memory_grow_a_stale_view_is_detected_and_reported() {
    let mut rt = Runtime::new();
    let (_, _, t) = packed(&mut rt, 10, everything(), u32::MAX);
    let epoch = rt.memory_epoch();
    assert_eq!(rt.view_current(t, epoch), Ok(()));
    rt.force_grow();
    assert_ne!(rt.memory_epoch(), epoch);
    assert_eq!(rt.view_current(t, epoch), Err(AbiError::StaleView));
    // reading the views again at the new epoch works
    assert_eq!(rt.view_current(t, rt.memory_epoch()), Ok(()));
    assert_eq!(AbiError::StaleView.code(), -2);
}

#[test]
fn issue_54_a_file_the_reader_refuses_is_an_error_code_and_leaks_no_handle() {
    let mut rt = Runtime::new();
    assert_eq!(rt.open(b"not a pvec file".to_vec()), Err(AbiError::Read));
    assert_eq!(AbiError::Read.code(), -4);
    let codes =
        [AbiError::Released, AbiError::StaleView, AbiError::BadArgument, AbiError::Read, AbiError::WrongKind];
    let mut seen: Vec<i32> = codes.iter().map(|c| c.code()).collect();
    assert!(seen.iter().all(|c| *c < 0));
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), codes.len());
}

/// What is wrong with a binding file: every export takes and returns numbers or a byte or number slice,
/// so no call can hand JS an object, an array of objects or a string per entity. Signatures are read
/// whole, wherever the formatter wrapped them.
fn surface_problems(source: &str) -> Vec<String> {
    const SCALARS: [&str; 3] = ["u32", "i32", "f64"];
    let code: String = source
        .lines()
        .filter(|l| !l.trim().starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = Vec::new();
    for banned in ["JsValue", "js_sys", "String", "Vec<", "Array", "Object", "Box<"] {
        if code.contains(banned) {
            out.push(format!("`{banned}` appears in the exports"));
        }
    }
    for part in code.split("pub fn ").skip(1) {
        let sig = part.split('{').next().unwrap_or("");
        let (args, ret) = match sig.split_once("->") {
            Some((a, r)) => (a, r.trim()),
            None => (sig, ""),
        };
        if !(ret.is_empty() || SCALARS.contains(&ret)) {
            out.push(format!("returns `{ret}`: {sig}"));
        }
        let inside = args.split_once('(').and_then(|(_, r)| r.rsplit_once(')')).map_or("", |(a, _)| a);
        for arg in inside.split(',').map(str::trim).filter(|a| !a.is_empty()) {
            let ty = arg.rsplit_once(':').map_or("", |(_, t)| t.trim());
            if !(SCALARS.contains(&ty) || ty == "&[u8]" || ty == "&[f64]") {
                out.push(format!("takes `{ty}`: {sig}"));
            }
        }
    }
    out
}

#[test]
fn issue_54_no_export_returns_an_object_an_array_of_objects_or_a_string() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bindings.rs");
    let source = fs::read_to_string(&path).expect("src/bindings.rs exists");
    assert_eq!(surface_problems(&source), Vec::<String>::new());
    for name in [
        "wasm_abi_version",
        "open",
        "query_viewport",
        "tessellate_batch",
        "release",
        "memory_epoch",
        "view_current",
    ] {
        assert!(source.contains(&format!("pub fn {name}(")), "the export `{name}` is missing");
    }
}

#[test]
fn issue_54_the_surface_check_catches_an_export_that_returns_strings_or_objects() {
    assert!(!surface_problems("pub fn names() -> Vec<String> {").is_empty());
    assert!(!surface_problems("pub fn entity(id: u32) -> JsValue {").is_empty());
    assert!(!surface_problems("pub fn label(id: u32) -> String {").is_empty());
    assert!(!surface_problems("pub fn open(bytes: Vec<u8>) -> i32 {").is_empty());
    assert!(surface_problems("pub fn open(bytes: &[u8]) -> i32 {").is_empty());
    assert!(!surface_problems("pub fn names(\n    n: u32,\n) -> Vec<String> {").is_empty(), "wrapped signatures are read whole");
}
