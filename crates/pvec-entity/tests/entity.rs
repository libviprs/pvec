//! File-local stable EntityId (P3.6).
#![allow(missing_docs)]

use pvec_entity::*;
use pvec_geometry::{Bounds, CurveKind, Point2};
use pvec_numeric::MAX_ENTITIES;

fn bounds(x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds {
    Bounds::new(Point2::new(x0, y0), Point2::new(x1, y1)).unwrap()
}

fn entry(i: usize) -> (CurveKind, Bounds) {
    let k = CurveKind::ALL[i % CurveKind::ALL.len()];
    let f = i as f64;
    (k, bounds(f, f, f + 1.0, f + 2.0))
}

// ---------------------------------------------------------------- the type

#[test]
fn an_entity_id_is_a_four_byte_integer_with_no_room_for_a_string_handle_or_pointer() {
    assert_eq!(core::mem::size_of::<EntityId>(), 4);
    assert_eq!(core::mem::align_of::<EntityId>(), 4);
    assert_eq!(core::mem::size_of::<Option<EntityId>>(), 8, "no niche is promised");
    // It is Copy, ordered and hashable: it lives in hot arrays and index entries.
    fn needs<T: Copy + Ord + core::hash::Hash + core::fmt::Debug>() {}
    needs::<EntityId>();
}

#[test]
fn ids_are_bounded_by_the_entity_limit() {
    assert!(EntityId::new(0).is_ok());
    assert!(EntityId::new(MAX_ENTITIES as u32 - 1).is_ok());
    assert_eq!(EntityId::new(MAX_ENTITIES as u32).unwrap_err(), EntityError::TooMany);
    assert_eq!(EntityId::new(u32::MAX).unwrap_err(), EntityError::TooMany);
    assert_eq!(EntityId::new(7).unwrap().index(), 7);
}

// ---------------------------------------------------------------- dense and exhaustive

#[test]
fn a_table_assigns_dense_ids_from_zero_in_push_order() {
    let mut t = EntityTable::new();
    for i in 0..50 {
        let (k, b) = entry(i);
        let id = t.push(k, b, Representation::Exact).unwrap();
        assert_eq!(id.index() as usize, i);
    }
    assert_eq!(t.len(), 50);
    let max = (0..t.len()).map(|i| t.id_at(i).unwrap().index()).max().unwrap();
    assert_eq!(max as usize + 1, t.len(), "the maximum id plus one is the entity count");
}

#[test]
fn a_gap_a_duplicate_or_a_missing_tail_fails_dense_validation() {
    let id = |n: u32| EntityId::new(n).unwrap();
    assert_eq!(check_dense(&[id(2), id(0), id(1)]), Ok(3), "order does not matter, coverage does");
    assert_eq!(check_dense(&[]), Ok(0));
    assert_eq!(check_dense(&[id(0), id(1), id(3)]), Err(EntityError::Gap(id(2))));
    assert_eq!(check_dense(&[id(0), id(0), id(1)]), Err(EntityError::Duplicate(id(0))));
    assert_eq!(check_dense(&[id(1), id(2)]), Err(EntityError::Gap(id(0))));
    // an id at or above the count cannot belong to a dense artifact
    assert_eq!(check_dense(&[id(0), id(5)]), Err(EntityError::Gap(id(1))));
}

#[test]
fn dense_validation_never_allocates_by_an_untrusted_id() {
    // An id near the limit in a two-entity list must not size a 2^28 bitmap; the count is the bound.
    let hostile = [EntityId::new(MAX_ENTITIES as u32 - 1).unwrap(), EntityId::new(0).unwrap()];
    assert!(check_dense(&hostile).is_err());
}

// ---------------------------------------------------------------- preview and exact share one identity

#[test]
fn a_preview_and_its_exact_replacement_carry_the_same_entity_id() {
    let mut t = EntityTable::new();
    let a = t.push(CurveKind::BSpline, bounds(0.0, 0.0, 10.0, 10.0), Representation::Preview).unwrap();
    let b = t.push(CurveKind::Line, bounds(1.0, 1.0, 2.0, 2.0), Representation::Exact).unwrap();
    assert_eq!(t.representation(a).unwrap(), Representation::Preview);
    let after = t.refine(a, bounds(0.0, 0.0, 9.5, 10.0)).unwrap();
    assert_eq!(after, a, "the swap resolves to one identity");
    assert_eq!(t.representation(a).unwrap(), Representation::Exact);
    assert_eq!(t.len(), 2, "refining adds no entity");
    assert_eq!(t.bounds(a).unwrap(), bounds(0.0, 0.0, 9.5, 10.0));
    assert_eq!(t.kind(a).unwrap(), CurveKind::BSpline, "the kind does not change either");
    assert_eq!(t.representation(b).unwrap(), Representation::Exact);
    // refining twice, or refining something that is already exact, is an error and changes nothing
    assert_eq!(t.refine(a, bounds(0.0, 0.0, 1.0, 1.0)).unwrap_err(), EntityError::AlreadyExact(a));
    assert_eq!(t.refine(b, bounds(0.0, 0.0, 1.0, 1.0)).unwrap_err(), EntityError::AlreadyExact(b));
    assert_eq!(t.bounds(a).unwrap(), bounds(0.0, 0.0, 9.5, 10.0));
}

// ---------------------------------------------------------------- determinism

#[test]
fn the_same_input_gives_the_same_ids_and_a_reordered_input_does_not() {
    let items: Vec<(CurveKind, Bounds)> = (0..40).map(entry).collect();
    let build = |order: &[usize]| {
        let mut t = EntityTable::new();
        let ids: Vec<(usize, EntityId)> = order
            .iter()
            .map(|&i| (i, t.push(items[i].0, items[i].1, Representation::Exact).unwrap()))
            .collect();
        (t, ids)
    };
    let natural: Vec<usize> = (0..40).collect();
    let (t1, ids1) = build(&natural);
    let (t2, ids2) = build(&natural);
    assert_eq!(ids1, ids2, "regenerating from the same input gives identical ids");
    assert_eq!(t1, t2);

    let mut swapped = natural.clone();
    swapped.swap(3, 17);
    let (_, ids3) = build(&swapped);
    let id_of = |ids: &[(usize, EntityId)], item: usize| ids.iter().find(|(i, _)| *i == item).unwrap().1;
    assert_ne!(id_of(&ids1, 3), id_of(&ids3, 3), "reordering the input changes the ids");
    assert_ne!(id_of(&ids1, 17), id_of(&ids3, 17));
    assert_eq!(id_of(&ids1, 5), id_of(&ids3, 5), "entities that did not move keep their id");
}

// ---------------------------------------------------------------- bounds without the payload

#[test]
fn an_id_resolves_to_bounds_without_any_geometry_payload() {
    // The table has no curves at all, only the bounds side table, and still answers.
    let mut t = EntityTable::new();
    for i in 0..100 {
        let (k, b) = entry(i);
        t.push(k, b, Representation::Exact).unwrap();
    }
    for i in [0u32, 1, 57, 99] {
        let id = EntityId::new(i).unwrap();
        assert_eq!(t.bounds(id).unwrap(), entry(i as usize).1);
    }
    let missing = EntityId::new(100).unwrap();
    assert_eq!(t.bounds(missing).unwrap_err(), EntityError::Unknown(missing));
    assert_eq!(t.kind(missing).unwrap_err(), EntityError::Unknown(missing));
    assert_eq!(t.representation(missing).unwrap_err(), EntityError::Unknown(missing));
}

#[test]
fn the_table_refuses_to_grow_past_the_entity_limit_before_allocating() {
    let t = EntityTable::new();
    assert_eq!(t.capacity_left(), MAX_ENTITIES);
    assert_eq!(EntityTable::with_expected(MAX_ENTITIES + 1).unwrap_err(), EntityError::TooMany);
    assert!(EntityTable::with_expected(10).is_ok());
}
