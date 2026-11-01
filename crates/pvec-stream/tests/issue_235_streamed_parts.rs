//! Issue #235: the pieces the streaming writer builds from. A directory built from entities that arrive already
//! sorted is the directory `build_directory` builds, and the Tier 0 plan of a leaf made from the outcome of each of
//! its entities is the plan `cut` gives, so the writer that never holds the drawing writes the same bytes.

use pvec_entity::EntityId;
use pvec_geometry::{Curve, Point2, Polyline};
use pvec_stream::{
    Aabb, DirectoryBuilder, EntityBox, Tier0Builder, Tier0Error, Tier0Outcome, Tier0Params, Tier0Plan,
    build_directory, entity_key, tier0_outcome,
};

fn id(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn boxes(n: u32, seed: u64) -> Vec<EntityBox> {
    let mut rng = Rng(seed | 1);
    // ids are not in order, so the order the builder sees is the sort's and not the input's
    let mut ids: Vec<u32> = (0..n).map(|i| i * 3 + 1).collect();
    for i in (1..ids.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        ids.swap(i, j);
    }
    ids.into_iter()
        .map(|i| {
            let (x, y) = (rng.unit() * 1000.0 - 500.0, rng.unit() * 1000.0);
            let (w, h) = (rng.unit() * 20.0, rng.unit() * 20.0);
            // some boxes share a centre, so ties on the key are decided by the id
            let (x, y) = if i % 7 == 0 { (10.0, 10.0) } else { (x, y) };
            EntityBox { id: id(i), bounds: Aabb::new([x, y], [x + w, y + h]).unwrap() }
        })
        .collect()
}

#[test]
fn a_directory_built_from_sorted_entities_is_the_directory_build_directory_builds() {
    for (n, capacity) in [(1, 1), (5, 2), (100, 7), (1000, 32), (1000, 100_000), (4097, 64)] {
        let entities = boxes(n, u64::from(n) * 31 + u64::from(capacity));
        let whole = build_directory(&entities, capacity).unwrap();
        let world = entities.iter().skip(1).fold(entities[0].bounds, |w, e| w.union(&e.bounds));
        let mut order: Vec<(u32, u32, usize)> = entities
            .iter()
            .enumerate()
            .map(|(i, e)| (entity_key(&world, &e.bounds), e.id.index(), i))
            .collect();
        order.sort_unstable();
        let mut builder = DirectoryBuilder::new(capacity).unwrap();
        let mut cells = vec![None; entities.len()];
        for (_, _, i) in order {
            cells[i] = Some(builder.push(&entities[i].bounds).unwrap());
        }
        let streamed = builder.finish().unwrap();
        assert_eq!(streamed.bytes, whole.bytes, "{n} entities in cells of {capacity}");
        assert_eq!((streamed.nodes, streamed.bounds), (whole.nodes, whole.bounds));
        assert_eq!(cells.into_iter().map(Option::unwrap).collect::<Vec<_>>(), whole.cells);
    }
}

#[test]
fn a_directory_builder_refuses_what_build_directory_refuses() {
    assert!(DirectoryBuilder::new(0).is_err());
    assert!(DirectoryBuilder::new(4).unwrap().finish().is_err(), "no entity");
}

fn curve(i: u32) -> Curve {
    let k = (i % 5) as usize;
    let n = [2, 3, 70, 200, 5][k];
    let pts: Vec<Point2> = (0..n)
        .map(|j| Point2 { x: f64::from(i) + f64::from(j as u32), y: (f64::from(j as u32) * 0.3).sin() * 4.0 })
        .collect();
    Curve::Polyline(Polyline::new(pts).unwrap())
}

#[test]
fn the_plan_of_a_leaf_from_outcomes_is_the_plan_cut_gives_and_an_outcome_survives_its_bytes() {
    let params = Tier0Params { max_error: 0.05 };
    let n = 60u32;
    let mut builder = Tier0Builder::new(n, params).unwrap();
    let mut outcomes = Vec::new();
    for i in 0..n {
        let c = curve(i);
        builder.add_curve(id(i), &c).unwrap();
        let o = tier0_outcome(id(i), &c, &params).unwrap();
        assert_eq!(Tier0Outcome::from_bytes(id(i), &o.to_bytes()).unwrap(), o, "entity {i}");
        outcomes.push((id(i), o));
    }
    let whole = builder.finish().unwrap();
    for ids in [vec![0u32, 1, 2], vec![59, 3, 10, 11, 12, 40], (0..n).collect(), vec![7], vec![2, 3, 4, 8, 9]] {
        let wanted: Vec<EntityId> = ids.iter().map(|i| id(*i)).collect();
        let cut = whole.cut(&wanted).unwrap().encode();
        let leaf: Vec<(EntityId, Tier0Outcome)> = ids.iter().map(|i| outcomes[*i as usize].clone()).collect();
        assert_eq!(Tier0Plan::leaf(n, leaf).unwrap().encode(), cut, "{ids:?}");
    }
    assert!(matches!(
        Tier0Plan::leaf(n, vec![outcomes[1].clone(), outcomes[1].clone()]),
        Err(Tier0Error::Duplicate(_))
    ));
}

#[test]
fn a_bound_that_is_not_one_is_refused_by_the_classifier_as_by_the_builder() {
    let c = curve(3);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let params = Tier0Params { max_error: bad };
        assert!(Tier0Builder::new(4, params).is_err());
        assert!(tier0_outcome(id(0), &c, &params).is_err(), "{bad}");
    }
}

#[test]
fn bytes_that_are_not_an_outcome_are_refused_and_not_a_panic() {
    for bytes in [&[][..], &[9], &[1], &[1, 9], &[2], &[2, 0, 0, 0], &[0, 0], &[2, 1, 0, 0, 0x80, 0x3f]] {
        assert!(Tier0Outcome::from_bytes(id(0), bytes).is_err(), "{bytes:?}");
    }
}
