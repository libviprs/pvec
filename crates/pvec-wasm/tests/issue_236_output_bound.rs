//! Issue #236: an absolute tessellation error of 1e-6 over a drawing about 130,000 units wide used to grow one
//! vector until `pvec-wasm` panicked with a capacity overflow. A batch is bounded now and the call returns a
//! typed refusal, code -11, that the caller can read. `docs/WASM_ABI.md` is the contract.
#![allow(missing_docs)]

use pvec::{Bounds2, EntityId, Transform2};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_wasm::{Precision, Runtime, WASM_ABI_VERSION};
use std::fs;
use std::path::Path;

/// The radius of a circle on a drawing about 130,000 units wide.
const RADIUS: f64 = 65_000.0;

/// `n` circles of radius 65,000, entity `i + 1` centred `i` units along x.
fn circles(n: u32) -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "issue-236".into(),
            producer_version: "1".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    for i in 0..n {
        let c = [f64::from(i), 0.0];
        let meta = EntityMeta {
            id: EntityId::new(i + 1).unwrap(),
            style: 0,
            cell: 0,
            bounds: Bounds { min: [c[0] - RADIUS, -RADIUS], max: [c[0] + RADIUS, RADIUS] },
        };
        w.push_circle_arc(meta, c, RADIUS, [0.0, std::f64::consts::TAU]).unwrap();
    }
    w.finish().unwrap()
}

/// An absolute error of 1e-6 and no screen budget worth the name, with the most segments the ABI takes.
fn tight() -> pvec::TessellationContext {
    pvec::TessellationContext {
        source_to_map: Transform2::IDENTITY,
        map_to_screen: Transform2::IDENTITY,
        pixel_ratio: 1.0,
        max_screen_error_px: f64::INFINITY,
        max_world_error: 1e-6,
        max_segments: u32::MAX as usize,
    }
}

fn all() -> Bounds2 {
    Bounds2 { min: [-1e9, -1e9], max: [1e9, 1e9] }
}

fn batch(rt: &mut Runtime, n: u32) -> u32 {
    let file = rt.open(circles(n)).unwrap();
    rt.query_viewport(file, all(), u32::MAX).unwrap()
}

#[test]
fn issue_236_a_batch_that_needs_more_vertices_than_the_bound_is_refused_with_code_minus_11() {
    // each circle needs 2^20 + 1 vertices at 1e-6, so twenty pass the bound of 2^24 (16.7 million)
    let mut rt = Runtime::new();
    let b = batch(&mut rt, 20);
    for precision in [Precision::F64, Precision::F32] {
        match rt.tessellate_batch(b, &tight(), precision) {
            Err(e) => assert_eq!(e.code(), -11, "{precision:?}"),
            Ok(_) => panic!("{precision:?}: twenty circles at 1e-6 were tessellated past the bound"),
        }
    }
}

#[test]
fn issue_236_the_refusal_leaves_the_module_usable() {
    let mut rt = Runtime::new();
    let big = batch(&mut rt, 20);
    assert!(rt.tessellate_batch(big, &tight(), Precision::F64).is_err());
    // the same batch with a budget that fits goes through, and so does a fresh one
    let mut loose = tight();
    loose.max_world_error = 1.0;
    let t = rt.tessellate_batch(big, &loose, Precision::F64).unwrap();
    assert_eq!(rt.entity_ids(t).unwrap().len(), 20);
    assert_eq!(rt.failed(t), Ok(0));
}

#[test]
fn issue_236_a_batch_under_the_bound_is_not_refused() {
    // one circle is 2^20 + 1 vertices, well under 2^24
    let mut rt = Runtime::new();
    let b = batch(&mut rt, 1);
    let t = rt.tessellate_batch(b, &tight(), Precision::F32).unwrap();
    let n = rt.offsets(t).unwrap()[1];
    assert!((1_000_000..1_100_000).contains(&n), "{n} vertices");
}

#[test]
fn issue_236_the_callers_own_max_segments_is_still_a_per_entity_failure_not_a_refusal() {
    let mut rt = Runtime::new();
    let b = batch(&mut rt, 3);
    let mut ctx = tight();
    ctx.max_segments = 1_000;
    let t = rt.tessellate_batch(b, &ctx, Precision::F64).unwrap();
    assert_eq!(rt.failed(t), Ok(3));
}

#[test]
fn issue_236_the_abi_stays_2_and_the_document_names_the_code_and_the_bound() {
    assert_eq!(WASM_ABI_VERSION, 2);
    let doc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/WASM_ABI.md")).unwrap();
    for needle in ["-11", "16,777,216", "OutputTooLarge"] {
        assert!(doc.contains(needle), "docs/WASM_ABI.md does not mention {needle}");
    }
}

#[test]
fn issue_236_one_curve_that_alone_needs_more_than_the_bound_is_refused_too() {
    // a radius of 1e9 at 1e-6 wants billions of segments; before the bound this one call tried to allocate them
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "issue-236".into(),
            producer_version: "1".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    let meta = EntityMeta {
        id: EntityId::new(1).unwrap(),
        style: 0,
        cell: 0,
        bounds: Bounds { min: [-1e9, -1e9], max: [1e9, 1e9] },
    };
    w.push_circle_arc(meta, [0.0, 0.0], 1e9, [0.0, std::f64::consts::TAU]).unwrap();
    let mut rt = Runtime::new();
    let file = rt.open(w.finish().unwrap()).unwrap();
    let b = rt.query_viewport(file, all(), u32::MAX).unwrap();
    assert_eq!(rt.tessellate_batch(b, &tight(), Precision::F64).map_err(|e| e.code()), Err(-11));
}
