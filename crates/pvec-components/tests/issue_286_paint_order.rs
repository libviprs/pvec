//! Capability 23, `paint_order` (issue 286, `docs/PAINT.md` section 2.2): the order the items of a page were
//! painted in, as runs of one kind. Written before the codec, so every test here starts red.
#![allow(missing_docs)]

use pvec_capabilities::{Class, Reader, Registry, Requirement, Version, ids};
use pvec_components::*;
use pvec_entity::EntityId;

fn run(kind: ItemKind, first: u32, count: u32) -> PaintRun {
    PaintRun { kind, first, count }
}

fn stream(runs: Vec<PaintRun>) -> PaintOrderStream {
    PaintOrderStream { runs }
}

fn rule_of(runs: Vec<PaintRun>) -> &'static str {
    stream(runs).encode().expect_err("refused").rule()
}

/// A payload by hand: the run count, then 12 bytes a run.
fn raw(runs: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut b = (runs.len() as u32).to_le_bytes().to_vec();
    for (k, f, c) in runs {
        b.extend_from_slice(&k.to_le_bytes());
        b.extend_from_slice(&f.to_le_bytes());
        b.extend_from_slice(&c.to_le_bytes());
    }
    b
}

fn rule_of_raw(runs: &[(u32, u32, u32)]) -> &'static str {
    PaintOrderStream::decode(&raw(runs)).expect_err("refused").rule()
}

// ---------------------------------------------------------------- registry

#[test]
fn the_registry_has_capability_23_as_an_optional_component_at_1_0_0() {
    let r = Registry::v1();
    let c = r.get(ids::PAINT_ORDER).expect("23 is registered");
    assert_eq!((c.id.get(), c.name), (23, "paint_order"));
    assert_eq!(c.class, Class::Component);
    assert_eq!(c.requirement, Requirement::Optional);
    assert_eq!(c.version, Version::new(1, 0, 0));
    assert_eq!(c.min_reader, Version::new(1, 0, 0), "a 1.0.0 reader still opens the file");
    assert!(c.iso_construct.is_empty());
}

// ---------------------------------------------------------------- wire

#[test]
fn a_run_is_twelve_bytes_after_the_u32_count() {
    let s = stream(vec![run(ItemKind::Stroke, 0, 1_000_000), run(ItemKind::Fill, 5, 2)]);
    let bytes = s.encode().unwrap();
    assert_eq!(bytes, raw(&[(1, 0, 1_000_000), (2, 5, 2)]));
    assert_eq!(bytes.len(), 4 + 12 * 2);
}

#[test]
fn the_four_kinds_round_trip_in_file_order() {
    let s = stream(vec![
        run(ItemKind::Fill, 0, 3),
        run(ItemKind::Stroke, 10, 4),
        run(ItemKind::Raster, 0, 1),
        run(ItemKind::Text, 7, 2),
        run(ItemKind::Fill, 3, 1),
    ]);
    assert_eq!(PaintOrderStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn an_empty_order_round_trips_and_is_four_bytes() {
    let s = stream(vec![]);
    let bytes = s.encode().unwrap();
    assert_eq!(bytes, vec![0, 0, 0, 0]);
    assert_eq!(PaintOrderStream::decode(&bytes).unwrap(), s);
}

#[test]
fn the_expanded_order_is_the_runs_in_file_order_and_items_ascending() {
    let s = stream(vec![run(ItemKind::Fill, 4, 2), run(ItemKind::Stroke, 0, 2), run(ItemKind::Fill, 0, 1)]);
    let items: Vec<(ItemKind, u32)> = s.items().collect();
    assert_eq!(
        items,
        vec![
            (ItemKind::Fill, 4),
            (ItemKind::Fill, 5),
            (ItemKind::Stroke, 0),
            (ItemKind::Stroke, 1),
            (ItemKind::Fill, 0)
        ]
    );
}

#[test]
fn a_million_strokes_in_one_run_cost_twelve_bytes() {
    let bytes = stream(vec![run(ItemKind::Stroke, 0, 1_000_000)]).encode().unwrap();
    assert_eq!(bytes.len(), 16);
}

// ---------------------------------------------------------------- the shape rules

#[test]
fn a_payload_that_ends_inside_a_run_is_comp_01() {
    let mut b = raw(&[(1, 0, 1)]);
    b.pop();
    assert_eq!(PaintOrderStream::decode(&b).unwrap_err().rule(), "COMP-01");
    // a count that promises more runs than there are bytes
    assert_eq!(PaintOrderStream::decode(&2u32.to_le_bytes()).unwrap_err().rule(), "COMP-01");
}

#[test]
fn bytes_after_the_last_run_are_comp_02() {
    let mut b = raw(&[(1, 0, 1)]);
    b.push(0);
    assert_eq!(PaintOrderStream::decode(&b).unwrap_err().rule(), "COMP-02");
}

#[test]
fn more_than_1048576_runs_are_comp_03_before_anything_is_allocated() {
    let b = (1_048_577u32).to_le_bytes().to_vec();
    assert_eq!(PaintOrderStream::decode(&b).unwrap_err().rule(), "COMP-03");
    let runs: Vec<PaintRun> = (0..1_048_577).map(|i| run(ItemKind::Stroke, i * 2, 1)).collect();
    assert_eq!(rule_of(runs), "COMP-03");
}

#[test]
fn a_run_of_zero_items_is_comp_05() {
    assert_eq!(rule_of_raw(&[(1, 0, 0)]), "COMP-05");
    assert_eq!(rule_of(vec![run(ItemKind::Fill, 0, 0)]), "COMP-05");
}

#[test]
fn a_kind_other_than_1_to_4_is_comp_11() {
    for k in [0, 5, 99, u32::MAX] {
        assert_eq!(rule_of_raw(&[(k, 0, 1)]), "COMP-11", "kind {k}");
    }
}

#[test]
fn an_overflowing_run_is_comp_08_for_strokes_and_comp_03_for_the_others() {
    assert_eq!(rule_of_raw(&[(1, u32::MAX, 2)]), "COMP-08");
    assert_eq!(rule_of_raw(&[(2, u32::MAX, 2)]), "COMP-03");
    assert_eq!(rule_of_raw(&[(3, u32::MAX - 1, 5)]), "COMP-03");
    assert_eq!(rule_of_raw(&[(4, u32::MAX, 2)]), "COMP-03");
    // a stroke run that stays under u32 but past the entity limit is the same rule
    assert_eq!(rule_of_raw(&[(1, 0xFFFF_FF00, 16)]), "COMP-08");
}

#[test]
fn an_expanded_list_over_2_pow_24_items_is_comp_03() {
    let ok = stream(vec![run(ItemKind::Fill, 0, 1 << 24)]);
    assert!(ok.encode().is_ok(), "exactly 2^24 items is allowed");
    assert_eq!(rule_of(vec![run(ItemKind::Fill, 0, 1 << 24), run(ItemKind::Text, 0, 1)]), "COMP-03");
    assert_eq!(rule_of(vec![run(ItemKind::Fill, 0, (1 << 24) + 1)]), "COMP-03");
}

// ---------------------------------------------------------------- COMP-21

#[test]
fn an_item_named_twice_in_one_run_set_is_comp_21() {
    // across runs of one kind, overlapping
    assert_eq!(rule_of(vec![run(ItemKind::Fill, 0, 5), run(ItemKind::Fill, 4, 2)]), "COMP-21");
    // the same single item twice
    assert_eq!(rule_of(vec![run(ItemKind::Text, 3, 1), run(ItemKind::Stroke, 0, 1), run(ItemKind::Text, 3, 1)]), "COMP-21");
    // one run inside another, in either order
    assert_eq!(rule_of(vec![run(ItemKind::Raster, 2, 1), run(ItemKind::Raster, 0, 9)]), "COMP-21");
    assert_eq!(rule_of_raw(&[(2, 0, 5), (2, 4, 2)]), "COMP-21");
}

#[test]
fn the_same_number_of_two_different_kinds_is_not_a_duplicate() {
    let s = stream(vec![
        run(ItemKind::Stroke, 0, 3),
        run(ItemKind::Fill, 0, 3),
        run(ItemKind::Raster, 0, 3),
        run(ItemKind::Text, 0, 3),
    ]);
    assert_eq!(PaintOrderStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn touching_runs_of_one_kind_are_not_a_duplicate() {
    let s = stream(vec![run(ItemKind::Fill, 5, 2), run(ItemKind::Stroke, 0, 1), run(ItemKind::Fill, 7, 2)]);
    assert!(s.encode().is_ok());
}

// ---------------------------------------------------------------- COMP-22, validation

fn exists(e: EntityId) -> bool {
    e.index() < 10
}

#[test]
fn validation_accepts_items_that_exist() {
    let s = stream(vec![
        run(ItemKind::Stroke, 0, 10),
        run(ItemKind::Fill, 0, 4),
        run(ItemKind::Raster, 0, 2),
        run(ItemKind::Text, 1, 2),
    ]);
    assert_eq!(s.validate(exists, 4, 2, 3), Ok(()));
}

#[test]
fn validation_refuses_a_stroke_that_is_not_drawn_as_comp_22() {
    let s = stream(vec![run(ItemKind::Stroke, 8, 3)]);
    let e = s.validate(exists, 0, 0, 0).unwrap_err();
    assert_eq!(e.rule(), "COMP-22");
    // an entity that is in the geometry but hidden is the same: the callback says it is not drawn
    let hidden_5 = |e: EntityId| e.index() < 10 && e.index() != 5;
    assert_eq!(stream(vec![run(ItemKind::Stroke, 4, 3)]).validate(hidden_5, 0, 0, 0).unwrap_err().rule(), "COMP-22");
}

#[test]
fn validation_refuses_a_fill_raster_or_text_past_the_end_of_its_stream() {
    assert_eq!(stream(vec![run(ItemKind::Fill, 3, 2)]).validate(exists, 4, 9, 9).unwrap_err().rule(), "COMP-22");
    assert_eq!(stream(vec![run(ItemKind::Raster, 0, 3)]).validate(exists, 9, 2, 9).unwrap_err().rule(), "COMP-22");
    assert_eq!(stream(vec![run(ItemKind::Text, 2, 1)]).validate(exists, 9, 9, 2).unwrap_err().rule(), "COMP-22");
    // the last one that exists is fine
    assert_eq!(stream(vec![run(ItemKind::Text, 1, 1)]).validate(exists, 9, 9, 2), Ok(()));
}

#[test]
fn an_empty_order_validates_against_anything() {
    assert_eq!(stream(vec![]).validate(|_| false, 0, 0, 0), Ok(()));
}

// ---------------------------------------------------------------- framing, and old readers

#[test]
fn the_stream_is_framed_with_the_registry_version_and_round_trips_in_a_set() {
    let r = Registry::v1();
    let s = stream(vec![run(ItemKind::Stroke, 0, 7), run(ItemKind::Fill, 0, 1)]);
    let framed = s.into_stream(&r).unwrap();
    assert_eq!((framed.capability, framed.version), (ids::PAINT_ORDER, Version::new(1, 0, 0)));
    let set = encode_streams(&[framed]).unwrap();
    let knows = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::PAINT_ORDER] };
    let opened = read_streams(&set, &r, &knows).unwrap();
    assert_eq!(opened.streams.len(), 1);
    assert_eq!(PaintOrderStream::decode(&opened.streams[0].payload).unwrap(), s);
}

#[test]
fn a_reader_that_does_not_implement_23_skips_it_without_reading_the_payload() {
    let r = Registry::v1();
    // the payload is garbage: a reader that skips by length must not care
    let framed = pvec_components::Stream {
        capability: ids::PAINT_ORDER,
        version: Version::new(1, 0, 0),
        payload: vec![0xFF; 40],
    };
    let set = encode_streams(&[framed]).unwrap();
    let old = Reader { version: Version::new(1, 0, 0), implemented: vec![ids::HATCH] };
    let opened = read_streams(&set, &r, &old).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped.len(), 1);
    assert_eq!((opened.skipped[0].capability, opened.skipped[0].reason), (ids::PAINT_ORDER, SkipReason::NotImplemented));
}

#[test]
fn a_reader_that_has_never_heard_of_23_skips_it_as_unknown() {
    // an old registry: the same one without the new entries
    let full = Registry::v1();
    let old_entries: Vec<_> = full.entries().iter().filter(|c| c.id.get() < 23).cloned().collect();
    let old_registry = Registry::new(old_entries, Vec::new()).unwrap();
    let framed = pvec_components::Stream {
        capability: ids::PAINT_ORDER,
        version: Version::new(1, 0, 0),
        payload: vec![0xFF; 40],
    };
    let set = encode_streams(&[framed]).unwrap();
    let old = Reader { version: Version::new(1, 2, 0), implemented: vec![ids::HATCH] };
    let opened = read_streams(&set, &old_registry, &old).unwrap();
    assert_eq!(opened.skipped[0].reason, SkipReason::UnknownCapability);
}
