//! Issue #203: a handle has to stay a non-negative `i32` for as long as a slot is reused, because the
//! exports return `i32` and the negative numbers are error codes (`docs/WASM_ABI.md`).

use pvec::{Bounds2, EntityId};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_wasm::{AbiError, Precision, Runtime};

fn one_line() -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata { producer_id: "issue-203".into(), producer_version: "1".into(), corpus_revision: None },
        bounds: None,
        capabilities: Vec::new(),
    });
    let (a, b) = ([0.0, 0.0], [1.0, 0.5]);
    let meta = EntityMeta { id: EntityId::new(1).unwrap(), style: 0, cell: 0, bounds: Bounds { min: a, max: b } };
    w.push_line(meta, a, b).unwrap();
    w.finish().unwrap()
}

const I32_MAX: u32 = i32::MAX as u32;

#[test]
fn issue_203_a_recycled_slot_never_returns_a_handle_that_reads_as_negative() {
    let bytes = one_line();
    let mut rt = Runtime::new();
    for round in 0..40_000u32 {
        let file = rt.open(bytes.clone()).unwrap();
        assert!(file <= I32_MAX, "round {round}: handle {file:#x} does not fit the i32 the export returns");
        rt.release(file).unwrap();
    }
}

#[test]
fn issue_203_a_released_handle_is_still_refused_after_many_reuses() {
    let bytes = one_line();
    let mut rt = Runtime::new();
    let first = rt.open(bytes.clone()).unwrap();
    rt.release(first).unwrap();
    for _ in 0..40_000u32 {
        let file = rt.open(bytes.clone()).unwrap();
        rt.release(file).unwrap();
    }
    assert_eq!(rt.release(first), Err(AbiError::Released));
}

#[test]
fn issue_203_results_and_batches_stay_in_range_too() {
    let bytes = one_line();
    let mut rt = Runtime::new();
    let file = rt.open(bytes).unwrap();
    let everything = Bounds2 { min: [-1e9, -1e9], max: [1e9, 1e9] };
    let ctx = pvec::TessellationContext {
        source_to_map: pvec::Transform2::IDENTITY,
        map_to_screen: pvec::Transform2::IDENTITY,
        pixel_ratio: 1.0,
        max_screen_error_px: 0.25,
        max_world_error: f64::INFINITY,
        max_segments: 64,
    };
    for round in 0..40_000u32 {
        let batch = rt.query_viewport(file, everything, 10).unwrap();
        let result = rt.tessellate_batch(batch, &ctx, Precision::F64).unwrap();
        assert!(batch <= I32_MAX && result <= I32_MAX, "round {round}: batch {batch:#x}, result {result:#x}");
        rt.release(result).unwrap();
        rt.release(batch).unwrap();
    }
}

#[test]
fn issue_203_a_released_handle_is_refused_for_32767_reuses_and_the_generation_then_wraps_to_zero() {
    let bytes = one_line();
    let mut rt = Runtime::new();
    let first = rt.open(bytes.clone()).unwrap();
    rt.release(first).unwrap();
    // The generation is 15 bits, so the slot's handle comes round again after 32768 reuses.
    for round in 1..=32_768u32 {
        let file = rt.open(bytes.clone()).unwrap();
        assert!(file <= I32_MAX, "round {round}: handle {file:#x}");
        if round == 32_768 {
            assert_eq!(file, first, "the generation wraps at 0x8000, back to the first handle");
        } else {
            assert_ne!(file, first, "round {round}: a stale handle must not come back early");
            assert_eq!(rt.release(first), Err(AbiError::Released), "round {round}");
        }
        rt.release(file).unwrap();
    }
}
