//! The semantic class stream (P3.9): a capability of its own that says what a drawn thing is in the
//! building (wall, column) and which regions are rooms and corridors, without touching the geometry.
//! One test group per acceptance criterion of the issue.
#![allow(missing_docs)]

use pvec_capabilities::{CapabilityId, Class, Reader, Registry, Requirement, Version, ids};
use pvec_codec::{decode, encode};
use pvec_components::*;
use pvec_entity::EntityId;
use pvec_geometry::*;
use std::collections::HashMap;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

// ---------------------------------------------------------------- the fixture

/// A straight segment as a trimmed line.
fn seg(a: (f64, f64), b: (f64, f64)) -> Curve {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    Curve::Trimmed(
        Trimmed::new(
            Curve::Line(Line::new(Point2::new(a.0, a.1), Vector2::new(dx, dy)).unwrap()),
            0.0,
            len,
            true,
        )
        .unwrap(),
    )
}

/// A floor plan: a room (walls 0 to 3) with a round column (4) standing in it, and a corridor (walls 5
/// to 8) beside it. Wall 6 is drawn against the direction the corridor ring walks it.
fn geometry() -> Vec<(EntityId, Curve)> {
    let circle =
        Circle::new(Placement2::new(Point2::new(5.0, 4.0), Vector2::new(1.0, 0.0)).unwrap(), 0.5).unwrap();
    vec![
        (id(0), seg((0.0, 0.0), (10.0, 0.0))),
        (id(1), seg((10.0, 0.0), (10.0, 8.0))),
        (id(2), seg((10.0, 8.0), (0.0, 8.0))),
        (id(3), seg((0.0, 8.0), (0.0, 0.0))),
        (id(4), Curve::Circle(circle)),
        (id(5), seg((10.0, 0.0), (14.0, 0.0))),
        (id(6), seg((14.0, 8.0), (14.0, 0.0))),
        (id(7), seg((14.0, 8.0), (10.0, 8.0))),
        (id(8), seg((10.0, 8.0), (10.0, 0.0))),
    ]
}

struct Plan(HashMap<EntityId, Curve>);

impl Plan {
    fn new(block: &[u8]) -> Plan {
        Plan(decode(block).unwrap().into_iter().collect())
    }
}

impl EntityGeometry for Plan {
    fn endpoints(&self, e: EntityId) -> Option<([f64; 2], [f64; 2])> {
        let c = self.0.get(&e)?;
        let d = c.domain();
        let (a, b) = (c.eval(d.lo()).unwrap(), c.eval(d.hi()).unwrap());
        Some(([a.x, a.y], [b.x, b.y]))
    }
}

/// The caller supplies the closure tolerance; the stream crate has none of its own. Test coordinates are
/// tens of units and the only inexact endpoint is the end of a full circle, off by about 1e-16.
const TOL: f64 = 0.001;

fn rule(source: SourceTag) -> Origin {
    Origin { source, confidence: None }
}

fn fixture_stream() -> SemanticClassStream {
    let el = |e, class, origin| ElementEntry { entity: id(e), class, origin };
    SemanticClassStream {
        elements: vec![
            el(0, ElementClass::Wall, rule(SourceTag::LayerRule)),
            el(1, ElementClass::Wall, rule(SourceTag::LayerRule)),
            el(2, ElementClass::Wall, rule(SourceTag::LayerRule)),
            el(3, ElementClass::Wall, rule(SourceTag::LayerRule)),
            el(4, ElementClass::Column, rule(SourceTag::BlockRule)),
            el(5, ElementClass::Wall, Origin { source: SourceTag::Inferred, confidence: Some(0.8) }),
            el(6, ElementClass::Wall, rule(SourceTag::Explicit)),
            el(7, ElementClass::Other, rule(SourceTag::Explicit)),
        ],
        regions: vec![
            RegionRecord {
                kind: RegionKind::Room,
                origin: Origin { source: SourceTag::Inferred, confidence: Some(0.55) },
                outer: vec![id(0), id(1), id(2), id(3)],
                holes: vec![vec![id(4)]],
            },
            RegionRecord {
                kind: RegionKind::Corridor,
                origin: rule(SourceTag::Explicit),
                outer: vec![id(5), id(6), id(7), id(8)],
                holes: vec![],
            },
        ],
    }
}

// ---------------------------------------------------------------- raw bytes for crafted payloads

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// One region record by hand: kind, source, no confidence, ring kind, outer ids, no holes.
fn raw_region(ring_kind: u32, ring: &[u32]) -> Vec<u8> {
    let mut b = u32s(&[1, 4, 0]);
    b.extend_from_slice(&0f64.to_le_bytes());
    b.extend(u32s(&[ring_kind, ring.len() as u32]));
    b.extend(u32s(ring));
    b.extend(u32s(&[0]));
    b
}

fn raw_stream(regions: &[Vec<u8>]) -> Vec<u8> {
    let mut b = u32s(&[0, regions.len() as u32]);
    for r in regions {
        b.extend_from_slice(r);
    }
    b
}

fn raw_element(entity: u32, class: u32, source: u32, has_conf: u32, conf: f64) -> Vec<u8> {
    let mut b = u32s(&[entity, class, source, has_conf]);
    b.extend_from_slice(&conf.to_le_bytes());
    b
}

fn raw_elements(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut b = u32s(&[entries.len() as u32, 0]);
    for e in entries {
        b.extend_from_slice(e);
    }
    b
}

fn knows(all: &[CapabilityId]) -> Reader {
    Reader { version: Version::new(1, 0, 0), implemented: all.to_vec() }
}

// ---------------------------------------------------------------- registry: its own id and version

#[test]
fn the_semantic_class_stream_has_its_own_optional_component_entry_in_the_registry() {
    let r = Registry::v1();
    let c = r.get(ids::SEMANTIC_CLASS).expect("semantic_class is not registered");
    assert_eq!((c.name, c.id.get()), ("semantic_class", 20));
    assert_eq!(c.class, Class::Component);
    assert_eq!(c.requirement, Requirement::Optional);
    assert!(c.iso_construct.is_empty(), "no ISO construct");
    assert_eq!(c.version, Version::new(1, 0, 0));
    // it did not take an id from anything else
    let same: Vec<_> = r.entries().iter().filter(|e| e.id == ids::SEMANTIC_CLASS).collect();
    assert_eq!(same.len(), 1);
    assert_eq!(r.entries().len(), 20);
}

#[test]
fn the_stream_is_framed_with_the_registry_version_and_round_trips() {
    let r = Registry::v1();
    let s = fixture_stream();
    let framed = s.into_stream(&r).unwrap();
    assert_eq!(framed.capability, ids::SEMANTIC_CLASS);
    assert_eq!(framed.version, Version::new(1, 0, 0));
    let set = encode_streams(&[framed]).unwrap();
    let opened = read_streams(&set, &r, &knows(&[ids::SEMANTIC_CLASS])).unwrap();
    assert_eq!(opened.streams.len(), 1);
    assert_eq!(SemanticClassStream::decode(&opened.streams[0].payload).unwrap(), s);
}

#[test]
fn the_same_stream_always_encodes_to_the_same_bytes() {
    assert_eq!(fixture_stream().encode().unwrap(), fixture_stream().encode().unwrap());
    assert!(!fixture_stream().encode().unwrap().is_empty());
}

// ---------------------------------------------------------------- criterion 1: stripping it changes nothing
//
// PLACEHOLDER until P2.5 and P2.6 exist (pvec#41 carries the follow-up). The geometry gates are not
// there yet, so `verdict` is a stand-in: it decodes the codec block and takes every number a gate would
// look at. It cannot see the stream set, so on its own it would make "with", "without" and "damaged"
// compare a function of the same bytes with itself. What these tests really prove is narrower, and is
// asserted explicitly below:
//
// * the block bytes are never touched by opening, stripping or damaging a stream set (they are
//   compared byte for byte before and after, and the set shares no storage with them),
// * the stream set opens, or is refused, on its own terms,
// * a stream cannot declare a geometry capability id.
//
// When the gates exist, replace `verdict` with a real run that takes the set as input.

/// Every number a geometry gate looks at, as bits. It is handed the stream set so that the placeholder
/// has the shape of the real check (the set goes in) even though a codec block cannot be influenced by it.
fn verdict(block: &[u8], set: &[u8], reader: &Reader) -> Vec<u64> {
    let r = Registry::v1();
    // the set is opened first and may be refused or skipped; either way the geometry below is read from
    // the block alone
    let _ = read_streams(set, &r, reader);
    let mut out = Vec::new();
    for (eid, c) in decode(block).unwrap() {
        out.push(u64::from(eid.index()));
        let d = c.domain();
        for i in 0..=64 {
            let p = c.eval(d.lo() + d.width() * f64::from(i) / 64.0).unwrap();
            out.push(p.x.to_bits());
            out.push(p.y.to_bits());
        }
        let b = c.bounds().unwrap();
        out.extend([b.min().x.to_bits(), b.min().y.to_bits(), b.max().x.to_bits(), b.max().y.to_bits()]);
    }
    out
}

/// A copy of the payload of a framed stream, for finding it inside a set or a block.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() >= 8 && haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn placeholder_stripping_the_stream_leaves_the_geometry_verdict_identical() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let block_before = block.clone();
    let reader = knows(&[ids::LAYER, ids::SEMANTIC_CLASS]);

    let framed = fixture_stream().into_stream(&r).unwrap();
    let with_set = encode_streams(std::slice::from_ref(&framed)).unwrap();
    let other = Stream {
        capability: ids::LAYER,
        version: Version::new(1, 0, 0),
        payload: vec![0, 0, 0, 0, 0, 0, 0, 0],
    };
    let without_set = encode_streams(&[other]).unwrap();
    let empty_set = encode_streams(&[]).unwrap();

    let with = verdict(&block, &with_set, &reader);
    assert!(with.len() > 500);
    assert_eq!(verdict(&block, &without_set, &reader), with, "with the stream stripped");
    assert_eq!(verdict(&block, &empty_set, &reader), with, "with every stream stripped");

    // the parts that are real: the stream is carried apart from the geometry, in both directions
    assert!(contains(&with_set, &framed.payload), "the set carries the payload");
    assert!(!contains(&block, &framed.payload), "the geometry block does not carry the payload");
    assert!(!contains(&without_set, &framed.payload), "the stripped set does not");
    assert_eq!(block, block_before, "opening and stripping streams never rewrite the block");
    let opened = read_streams(&with_set, &r, &reader).unwrap();
    assert_eq!(opened.streams.len(), 1, "present");
    let stripped = read_streams(&without_set, &r, &reader).unwrap();
    assert!(stripped.streams.iter().all(|s| s.capability != ids::SEMANTIC_CLASS));

    // the stream cannot reach a curve: it is not a geometry capability and a frame cannot say it is
    assert_eq!(
        encode_streams(&[Stream {
            capability: ids::CIRCLE,
            version: Version::new(1, 0, 0),
            payload: framed.payload
        }])
        .unwrap_err(),
        StreamError::GeometryCapability(ids::CIRCLE)
    );
}

#[test]
fn placeholder_a_damaged_semantic_class_payload_cannot_move_the_geometry_verdict() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let block_before = block.clone();
    let reader = knows(&[ids::SEMANTIC_CLASS]);
    let good = encode_streams(&[fixture_stream().into_stream(&r).unwrap()]).unwrap();
    let baseline = verdict(&block, &good, &reader);
    let mut framed = fixture_stream().into_stream(&r).unwrap();
    framed.payload = vec![0xff; 41];
    let damaged = encode_streams(&[framed]).unwrap();
    assert_eq!(verdict(&block, &damaged, &reader), baseline);
    assert_eq!(block, block_before);
    // a reader that knows nothing steps over it, reported, and the set still opens
    let opened = read_streams(&damaged, &r, &knows(&[])).unwrap();
    assert_eq!(opened.skipped.len(), 1);
    assert_eq!(opened.skipped[0].capability, ids::SEMANTIC_CLASS);
    assert_eq!(opened.skipped[0].reason, SkipReason::NotImplemented);
}

// ---------------------------------------------------------------- a damaged payload at a version the reader knows
//
// The rule (P3.9, docs/COMPONENT_STREAMS.md): the frame hands a known, compatible payload over undecoded,
// so a damaged one is found when the semantic reader decodes it. That error fails this stream only. It is
// reported (`SemanticRead::Damaged`), the set still opened, the other streams are intact, and the geometry
// is never involved. It does not fail the artifact and it is not silently dropped.

fn damaged_set(payload: Vec<u8>, with_layer: bool) -> Vec<u8> {
    let mut streams =
        vec![Stream { capability: ids::SEMANTIC_CLASS, version: Version::new(1, 0, 0), payload }];
    if with_layer {
        streams.push(Stream {
            capability: ids::LAYER,
            version: Version::new(1, 0, 0),
            payload: LayerStream { names: vec![], assignments: vec![] }.encode().unwrap(),
        });
    }
    encode_streams(&streams).unwrap()
}

#[test]
fn a_reader_that_knows_the_semantic_class_stream_reports_a_damaged_v1_payload_and_fails_only_that_stream() {
    let r = Registry::v1();
    let reader = knows(&[ids::SEMANTIC_CLASS, ids::LAYER]);
    let block = encode(&geometry()).unwrap().bytes;
    let good = fixture_stream().encode().unwrap();
    let cut = good[..good.len() - 3].to_vec();
    let mut trailing = good.clone();
    trailing.push(0);
    let mut bad_tag = good.clone();
    bad_tag[12..16].copy_from_slice(&9u32.to_le_bytes()); // the first element's class, undefined
    let cases: Vec<(Vec<u8>, ComponentError)> = vec![
        (cut, ComponentError::Truncated),
        (trailing, ComponentError::Trailing),
        (bad_tag, ComponentError::BadTag(9)),
        (vec![0xff; 41], ComponentError::TooMany),
    ];
    for (payload, expected) in cases {
        let set = damaged_set(payload, true);
        // the set opens: the damaged payload is handed over undecoded, it is not a framing error
        let opened = read_streams(&set, &r, &reader).expect("a damaged payload does not fail the set");
        assert_eq!(opened.streams.len(), 2);
        assert!(opened.skipped.is_empty());
        // decoding reports it, for this stream only
        assert_eq!(SemanticClassStream::from_opened(&opened), SemanticRead::Damaged(expected));
        // the sibling stream and the geometry are untouched
        let layer = opened.streams.iter().find(|s| s.capability == ids::LAYER).unwrap();
        assert!(LayerStream::decode(&layer.payload).is_ok());
        assert_eq!(decode(&block).unwrap().len(), 9);
    }
}

#[test]
fn from_opened_reads_a_good_stream_and_reports_absence_for_a_skipped_or_missing_one() {
    let r = Registry::v1();
    let reader = knows(&[ids::SEMANTIC_CLASS]);
    let set = encode_streams(&[fixture_stream().into_stream(&r).unwrap()]).unwrap();
    let opened = read_streams(&set, &r, &reader).unwrap();
    assert_eq!(SemanticClassStream::from_opened(&opened), SemanticRead::Present(fixture_stream()));
    // a reader without support, and a set without the stream, are both just absent
    let opened = read_streams(&set, &r, &knows(&[])).unwrap();
    assert_eq!(SemanticClassStream::from_opened(&opened), SemanticRead::Absent);
    let none = read_streams(&encode_streams(&[]).unwrap(), &r, &reader).unwrap();
    assert_eq!(SemanticClassStream::from_opened(&none), SemanticRead::Absent);
    // a newer major is skipped, so it is absent and never decoded (garbage in it is not looked at)
    let newer =
        Stream { capability: ids::SEMANTIC_CLASS, version: Version::new(2, 0, 0), payload: vec![0xa5; 97] };
    let opened = read_streams(&encode_streams(&[newer]).unwrap(), &r, &reader).unwrap();
    assert_eq!(SemanticClassStream::from_opened(&opened), SemanticRead::Absent);
}

// ---------------------------------------------------------------- criterion 2: a newer version still opens

#[test]
fn an_unknown_newer_semantic_class_version_opens_with_the_stream_reported_skipped() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let newer = Stream {
        capability: ids::SEMANTIC_CLASS,
        version: Version::new(2, 0, 0),
        payload: vec![0xa5; 97], // a layout this reader has never seen: it must not be looked at
    };
    let set = encode_streams(&[newer]).unwrap();
    let opened = read_streams(&set, &r, &knows(&[ids::SEMANTIC_CLASS])).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped.len(), 1);
    assert_eq!(opened.skipped[0].capability, ids::SEMANTIC_CLASS);
    assert_eq!(opened.skipped[0].version, Version::new(2, 0, 0));
    assert_eq!(opened.skipped[0].reason, SkipReason::IncompatibleVersion);
    assert_eq!(decode(&block).unwrap().len(), 9, "the geometry still opens");
}

// ---------------------------------------------------------------- criterion 3: a bump leaves geometry alone

#[test]
fn bumping_the_semantic_class_version_leaves_every_geometry_capability_version_unchanged() {
    let before = Registry::v1();
    let mut after = Registry::v1();
    after.bump_component_version(ids::SEMANTIC_CLASS, Version::new(2, 0, 0)).unwrap();
    assert_eq!(after.get(ids::SEMANTIC_CLASS).unwrap().version, Version::new(2, 0, 0));
    for (a, b) in before.entries().iter().zip(after.entries()) {
        if a.id != ids::SEMANTIC_CLASS {
            assert_eq!(a, b, "{} changed", a.name);
        }
    }
    // and the bump goes only upwards
    assert!(after.bump_component_version(ids::SEMANTIC_CLASS, Version::new(1, 5, 0)).is_err());
}

// ---------------------------------------------------------------- criteria 4 and 5: regions fail validation

#[test]
fn the_fixture_validates_including_a_wall_drawn_against_the_ring_direction_and_a_round_hole() {
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    assert_eq!(fixture_stream().validate(&plan, TOL), Ok(()));
    // a ring may start on an edge that is drawn against its walk: it closes only by reversing that first edge
    let mut rotated = fixture_stream();
    rotated.regions[1].outer = vec![id(6), id(7), id(8), id(5)];
    assert_eq!(rotated.validate(&plan, TOL), Ok(()));
    // and the same after a trip through the bytes
    let back = SemanticClassStream::decode(&fixture_stream().encode().unwrap()).unwrap();
    assert_eq!(back.validate(&plan, TOL), Ok(()));
}

#[test]
fn a_region_whose_boundary_is_not_closed_fails_validation() {
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    // three of the four walls: the ring stops short of where it began
    let mut s = fixture_stream();
    s.regions[0].outer = vec![id(0), id(1), id(2)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::NotClosed { region: 0 }));
    // walls that do not touch each other
    let mut s = fixture_stream();
    s.regions[1].outer = vec![id(5), id(7), id(6), id(8)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::NotClosed { region: 1 }));
    // a single open segment is not a ring, a lone circle is
    let mut s = fixture_stream();
    s.regions[0].outer = vec![id(0)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::NotClosed { region: 0 }));
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![id(5)]]; // an open segment, and not one the outer ring already uses
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::NotClosed { region: 0 }));
}

#[test]
fn a_ring_cannot_walk_the_same_entity_twice_even_when_it_closes_on_itself() {
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    // [A, A] with A an open segment: out along A and back along A ends where it began, which is a
    // zero-area back and forth and not a boundary
    let mut s = fixture_stream();
    s.regions[0].outer = vec![id(0), id(0)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(0) }));
    // a repeat that is not adjacent is a repeat too
    let mut s = fixture_stream();
    s.regions[0].outer = vec![id(0), id(1), id(0), id(1), id(2), id(3)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(0) }));
    // a closed circle as a ring of one is still fine
    let mut s = fixture_stream();
    s.regions[0].outer = vec![id(4)];
    s.regions[0].holes = vec![];
    assert_eq!(s.validate(&plan, TOL), Ok(()));
    // and the same circle twice is not
    s.regions[0].outer = vec![id(4), id(4)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(4) }));
    // within a hole
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![id(4), id(4)]];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(4) }));
    // the region index is the region's
    let mut s = fixture_stream();
    s.regions[1].outer = vec![id(5), id(5)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 1, entity: id(5) }));
}

#[test]
fn an_entity_cannot_be_shared_between_the_outer_ring_and_a_hole_or_between_two_holes() {
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![id(4)], vec![id(4)]];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(4) }));
    // outer and hole: the room's own wall used as a hole boundary
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![id(0), id(1), id(2), id(3)]];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(0) }));
    // the circle as both the outer ring and its own hole
    let mut s = fixture_stream();
    s.regions[0].outer = vec![id(4)];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::DuplicateEntity { region: 0, entity: id(4) }));
    // two different regions may share an entity: a wall between a room and a corridor serves both
    let mut s = fixture_stream();
    s.regions[1].outer = vec![id(5), id(6), id(7), id(1)];
    assert_eq!(s.validate(&plan, TOL), Ok(()));
}

#[test]
fn a_tolerance_that_is_not_finite_and_non_negative_is_refused() {
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -TOL, -f64::MIN_POSITIVE] {
        assert_eq!(fixture_stream().validate(&plan, bad), Err(RegionError::BadTolerance), "{bad}");
    }
    // infinity must not switch the closure rule off
    let mut open = fixture_stream();
    open.regions[0].outer = vec![id(0), id(1), id(2)];
    assert_eq!(open.validate(&plan, f64::INFINITY), Err(RegionError::BadTolerance));
    // zero is a legal, exact tolerance (the corridor's coordinates are exact; the circle's end is not)
    let mut corridor = fixture_stream();
    corridor.regions.remove(0);
    assert_eq!(corridor.validate(&plan, 0.0), Ok(()));
    assert_eq!(RegionError::BadTolerance.rule(), "COMP-19");
    assert_eq!(RegionError::DuplicateEntity { region: 0, entity: id(0) }.rule(), "COMP-18");
}

#[test]
fn a_region_that_references_a_missing_entity_fails_validation() {
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    let mut s = fixture_stream();
    s.regions[0].outer[2] = id(900);
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::MissingEntity { entity: id(900), region: Some(0) }));
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![id(901)]];
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::MissingEntity { entity: id(901), region: Some(0) }));
    // the region index is the region's, not a constant: the same failure in region 1 says 1
    let mut s = fixture_stream();
    s.regions[1].outer[0] = id(903);
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::MissingEntity { entity: id(903), region: Some(1) }));
    // an element class on an entity that is not there is refused too, and it belongs to no region
    let mut s = fixture_stream();
    s.elements[0].entity = id(902);
    assert_eq!(s.validate(&plan, TOL), Err(RegionError::MissingEntity { entity: id(902), region: None }));
}

#[test]
fn a_region_that_embeds_curve_parameters_fails_validation() {
    // ring kind 1 is references; 2 is the word an inline curve would use, and it is refused as geometry
    let ok = raw_stream(&[raw_region(1, &[0, 1, 2, 3])]);
    assert!(SemanticClassStream::decode(&ok).is_ok());
    let inline = raw_stream(&[raw_region(2, &[0, 1, 2, 3])]);
    assert_eq!(SemanticClassStream::decode(&inline).unwrap_err(), ComponentError::EmbeddedGeometry);
    let odd = raw_stream(&[raw_region(7, &[0, 1, 2, 3])]);
    assert_eq!(SemanticClassStream::decode(&odd).unwrap_err(), ComponentError::BadTag(7));
}

#[test]
fn a_hole_that_embeds_curve_parameters_fails_validation_too() {
    let mut region = u32s(&[1, 4, 0]);
    region.extend_from_slice(&0f64.to_le_bytes());
    region.extend(u32s(&[1, 1, 0, 1, 2, 1, 4])); // one outer id, one hole of kind 2 (inline) with one id
    assert_eq!(
        SemanticClassStream::decode(&raw_stream(&[region])).unwrap_err(),
        ComponentError::EmbeddedGeometry
    );
}

// ---------------------------------------------------------------- the enum is closed, the tags are explicit

#[test]
fn an_undefined_class_region_kind_or_source_tag_is_refused_never_carried_as_a_vendor_value() {
    let bad_class = raw_elements(&[raw_element(0, 9, 1, 0, 0.0)]);
    assert_eq!(SemanticClassStream::decode(&bad_class).unwrap_err(), ComponentError::BadTag(9));
    let bad_source = raw_elements(&[raw_element(0, 1, 0, 0, 0.0)]);
    assert_eq!(SemanticClassStream::decode(&bad_source).unwrap_err(), ComponentError::BadTag(0));
    let mut bad_kind = u32s(&[9, 4, 0]);
    bad_kind.extend_from_slice(&0f64.to_le_bytes());
    bad_kind.extend(u32s(&[1, 1, 0, 0]));
    assert_eq!(SemanticClassStream::decode(&raw_stream(&[bad_kind])).unwrap_err(), ComponentError::BadTag(9));
}

#[test]
fn absence_of_a_class_is_explicit() {
    let s = SemanticClassStream {
        elements: vec![ElementEntry {
            entity: id(3),
            class: ElementClass::Other,
            origin: rule(SourceTag::Explicit),
        }],
        regions: vec![],
    };
    let back = SemanticClassStream::decode(&s.encode().unwrap()).unwrap();
    assert_eq!(back.elements[0].class, ElementClass::Other);
}

#[test]
fn confidence_belongs_to_inferred_entries_only_and_lies_in_zero_to_one() {
    let el = |source, confidence| SemanticClassStream {
        elements: vec![ElementEntry {
            entity: id(0),
            class: ElementClass::Wall,
            origin: Origin { source, confidence },
        }],
        regions: vec![],
    };
    assert!(el(SourceTag::Inferred, Some(0.0)).encode().is_ok());
    assert!(el(SourceTag::Inferred, Some(1.0)).encode().is_ok());
    assert!(el(SourceTag::Inferred, None).encode().is_ok(), "an inferred entry may carry none");
    for bad in [-0.1, 1.5, f64::NAN, f64::INFINITY] {
        assert_eq!(
            el(SourceTag::Inferred, Some(bad)).encode().unwrap_err(),
            ComponentError::BadNumber,
            "{bad}"
        );
    }
    assert!(el(SourceTag::LayerRule, Some(0.5)).encode().is_err());
    assert!(el(SourceTag::Explicit, Some(0.5)).encode().is_err());
    // and on the way in
    assert!(
        SemanticClassStream::decode(&raw_elements(&[raw_element(0, 1, 1, 1, 0.5)])).is_err(),
        "layer-rule with confidence"
    );
    assert_eq!(
        SemanticClassStream::decode(&raw_elements(&[raw_element(0, 1, 3, 1, 2.0)])).unwrap_err(),
        ComponentError::BadNumber
    );
    assert!(SemanticClassStream::decode(&raw_elements(&[raw_element(0, 1, 3, 1, 0.5)])).is_ok());
    assert!(SemanticClassStream::decode(&raw_elements(&[raw_element(0, 1, 3, 0, 0.0)])).is_ok());
    assert!(
        SemanticClassStream::decode(&raw_elements(&[raw_element(0, 1, 3, 0, 0.5)])).is_err(),
        "bits where there is no confidence"
    );
}

#[test]
fn an_entity_is_classified_once_and_a_ring_is_never_empty_or_oversized() {
    let dup = SemanticClassStream {
        elements: vec![
            ElementEntry { entity: id(1), class: ElementClass::Wall, origin: rule(SourceTag::Explicit) },
            ElementEntry { entity: id(1), class: ElementClass::Column, origin: rule(SourceTag::Explicit) },
        ],
        regions: vec![],
    };
    assert_eq!(dup.encode().unwrap_err(), ComponentError::Duplicate(id(1)));
    let two = raw_elements(&[raw_element(1, 1, 4, 0, 0.0), raw_element(1, 2, 4, 0, 0.0)]);
    assert_eq!(SemanticClassStream::decode(&two).unwrap_err(), ComponentError::Duplicate(id(1)));

    let mut s = fixture_stream();
    s.regions[0].outer.clear();
    assert_eq!(s.encode().unwrap_err(), ComponentError::BadCount);
    assert_eq!(
        SemanticClassStream::decode(&raw_stream(&[raw_region(1, &[])])).unwrap_err(),
        ComponentError::BadCount
    );

    let mut s = fixture_stream();
    s.regions[0].outer = (0..=MAX_BOUNDARY as u32).map(id).collect();
    assert_eq!(s.encode().unwrap_err(), ComponentError::TooMany);
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![id(4)]; MAX_HOLES + 1];
    assert_eq!(s.encode().unwrap_err(), ComponentError::TooMany);
    let mut s = fixture_stream();
    s.regions[0].holes = vec![vec![]];
    assert_eq!(s.encode().unwrap_err(), ComponentError::BadCount);
}

#[test]
fn a_region_with_more_holes_than_the_limit_is_refused_on_decode_as_well() {
    let mut region = u32s(&[1, 4, 0]);
    region.extend_from_slice(&0f64.to_le_bytes());
    region.extend(u32s(&[1, 1, 0, (MAX_HOLES + 1) as u32]));
    for _ in 0..=MAX_HOLES {
        region.extend(u32s(&[1, 1, 4]));
    }
    assert_eq!(SemanticClassStream::decode(&raw_stream(&[region])).unwrap_err(), ComponentError::TooMany);
}

#[test]
fn every_payload_is_consumed_exactly_and_every_truncation_is_refused() {
    let bytes = fixture_stream().encode().unwrap();
    assert_eq!(SemanticClassStream::decode(&bytes).unwrap(), fixture_stream());
    for cut in 0..bytes.len() {
        assert!(SemanticClassStream::decode(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(SemanticClassStream::decode(&longer).unwrap_err(), ComponentError::Trailing);
}

#[test]
fn a_count_larger_than_the_bytes_left_is_refused_before_it_sizes_an_allocation() {
    assert!(SemanticClassStream::decode(&fixture_stream().encode().unwrap()).is_ok(), "control");
    let huge = u32s(&[u32::MAX, 0]);
    assert!(SemanticClassStream::decode(&huge).is_err());
    let huge_regions = u32s(&[0, 1 << 20]);
    assert!(SemanticClassStream::decode(&huge_regions).is_err());
    let mut region = u32s(&[1, 4, 0]);
    region.extend_from_slice(&0f64.to_le_bytes());
    region.extend(u32s(&[1, 4096])); // 4096 ids promised, none present
    assert_eq!(SemanticClassStream::decode(&raw_stream(&[region])).unwrap_err(), ComponentError::Truncated);
}

#[test]
fn random_corruption_never_panics() {
    let bytes = fixture_stream().encode().unwrap();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let (mut refused, mut changed) = (0, 0);
    for _ in 0..3000 {
        let mut b = bytes.clone();
        for _ in 0..(1 + next() % 4) {
            let at = (next() as usize) % b.len();
            b[at] = next() as u8;
        }
        match SemanticClassStream::decode(&b) {
            Err(_) => refused += 1,
            Ok(s) => {
                // whatever gets through is a well-formed stream: it encodes back to exactly these bytes
                assert_eq!(s.encode().unwrap(), b, "an accepted payload is canonical");
            }
        }
        if b != bytes {
            changed += 1;
        }
    }
    // a single changed byte lands in a count, a tag or an id table most of the time, so most mutants
    // must be refused; the seed is fixed, 93 percent are refused today and the floor is 85 percent, far above
    // the old floor of 100 in 3000
    assert!(changed > 2900, "the mutator changes the bytes ({changed})");
    assert!(
        refused * 100 > changed * 85,
        "corruption is detected often enough to mean something ({refused} of {changed})"
    );
}

// ---------------------------------------------------------------- criterion 6: a reader without support

#[test]
fn a_reader_with_no_semantic_support_still_reaches_display_fidelity_on_the_fixture() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let set = encode_streams(&[fixture_stream().into_stream(&r).unwrap()]).unwrap();
    // it knows every other stream, but not this one
    let reader = knows(&[ids::LAYER, ids::LINE_STYLE, ids::TEXT, ids::HATCH, ids::SOURCE_PROVENANCE]);
    let opened = read_streams(&set, &r, &reader).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped.len(), 1, "reported skipped, not an error");
    assert_eq!(opened.skipped[0].reason, SkipReason::NotImplemented);
    // all the walls, the column and the corridor linework evaluate
    let curves = decode(&block).unwrap();
    assert_eq!(curves.len(), 9);
    for (_, c) in curves {
        assert!(c.eval(c.domain().lo()).is_ok());
        assert!(c.bounds().is_ok());
    }
}

// ---------------------------------------------------------------- criterion 7: filter by class

#[test]
fn the_fixture_can_be_filtered_by_class() {
    let r = Registry::v1();
    let opened = {
        let set = encode_streams(&[fixture_stream().into_stream(&r).unwrap()]).unwrap();
        read_streams(&set, &r, &knows(&[ids::SEMANTIC_CLASS])).unwrap()
    };
    let s = SemanticClassStream::decode(&opened.streams[0].payload).unwrap();
    assert_eq!(s.entities_of(ElementClass::Wall), vec![id(0), id(1), id(2), id(3), id(5), id(6)]);
    assert_eq!(s.entities_of(ElementClass::Column), vec![id(4)]);
    assert_eq!(s.entities_of(ElementClass::Other), vec![id(7)]);
    let rooms = s.regions_of(RegionKind::Room);
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].outer, vec![id(0), id(1), id(2), id(3)]);
    assert_eq!(rooms[0].holes, vec![vec![id(4)]]);
    assert_eq!(s.regions_of(RegionKind::Corridor).len(), 1);
    // "show only the walls": the filtered entities are real curves in the geometry
    let plan = Plan::new(&encode(&geometry()).unwrap().bytes);
    for e in s.entities_of(ElementClass::Wall) {
        assert!(plan.endpoints(e).is_some());
    }
}

#[test]
fn the_source_tag_tells_a_drafted_wall_from_a_guessed_room() {
    let s = fixture_stream();
    let back = SemanticClassStream::decode(&s.encode().unwrap()).unwrap();
    assert_eq!(back.elements[0].origin.source, SourceTag::LayerRule);
    assert_eq!(back.elements[4].origin.source, SourceTag::BlockRule);
    assert_eq!(back.regions[0].origin, Origin { source: SourceTag::Inferred, confidence: Some(0.55) });
    assert_eq!(back.regions[1].origin.source, SourceTag::Explicit);
}
