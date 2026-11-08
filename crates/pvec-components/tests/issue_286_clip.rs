//! Capability 26, `clip` (issue 286, `docs/PAINT.md` section 2.5): clip regions made of rings of entities, nested
//! up to eight deep, and the paint items each one applies to. Written before the codec, so every test starts red.
#![allow(missing_docs)]

use pvec_capabilities::{Class, Reader, Registry, Requirement, Version, ids};
use pvec_components::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn clip(rule: FillRule, parent: Option<u32>, ring: &[u32]) -> ClipRecord {
    ClipRecord { rule, parent, boundary: ring.iter().map(|n| id(*n)).collect() }
}

fn run(kind: ItemKind, first: u32, count: u32, clip: u32) -> ClipRun {
    ClipRun { kind, first, count, clip }
}

fn fixture() -> ClipStream {
    ClipStream {
        clips: vec![
            clip(FillRule::NonZero, None, &[10, 11, 12]),
            clip(FillRule::EvenOdd, Some(0), &[20]),
            clip(FillRule::NonZero, None, &[30, 31]),
        ],
        runs: vec![
            run(ItemKind::Fill, 0, 3, 0),
            run(ItemKind::Stroke, 5, 2, 1),
            run(ItemKind::Raster, 0, 1, 2),
            run(ItemKind::Text, 4, 1, 1),
        ],
    }
}

fn rule_of(s: ClipStream) -> &'static str {
    s.encode().expect_err("refused").rule()
}

/// A chain of `n` clips, each the parent of the next.
fn chain(n: u32) -> ClipStream {
    ClipStream {
        clips: (0..n).map(|i| clip(FillRule::NonZero, i.checked_sub(1), &[i])).collect(),
        runs: vec![run(ItemKind::Fill, 0, 1, n - 1)],
    }
}

#[test]
fn the_registry_has_capability_26_as_an_optional_component_at_1_0_0() {
    let r = Registry::v1();
    let c = r.get(ids::CLIP).expect("26 is registered");
    assert_eq!((c.id.get(), c.name), (26, "clip"));
    assert_eq!((c.class, c.requirement), (Class::Component, Requirement::Optional));
    assert_eq!((c.version, c.min_reader), (Version::new(1, 0, 0), Version::new(1, 0, 0)));
}

#[test]
fn it_round_trips() {
    let s = fixture();
    assert_eq!(ClipStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn the_layout_is_two_counts_then_clips_then_16_byte_runs() {
    let s = ClipStream {
        clips: vec![clip(FillRule::EvenOdd, None, &[7, 8])],
        runs: vec![run(ItemKind::Fill, 2, 3, 0)],
    };
    let b = s.encode().unwrap();
    // counts 8, a clip of rule + 3 reserved + parent + kind + count + two ids = 4 + 4 + 4 + 4 + 8, a run of 16
    assert_eq!(b.len(), 8 + 24 + 16);
    assert_eq!(&b[..8], &[1, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(&b[8..12], &[1, 0, 0, 0], "even-odd, three reserved bytes");
    assert_eq!(&b[12..16], &[0xFF; 4], "no parent");
    assert_eq!(&b[16..24], &[1, 0, 0, 0, 2, 0, 0, 0], "references, two of them");
    assert_eq!(&b[32..], &[2, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn an_empty_stream_is_eight_bytes() {
    let s = ClipStream { clips: vec![], runs: vec![] };
    assert_eq!(s.encode().unwrap(), vec![0; 8]);
    assert_eq!(ClipStream::decode(&[0; 8]).unwrap(), s);
}

#[test]
fn the_clip_of_an_item_and_the_chain_from_the_outermost_are_found() {
    let s = fixture();
    assert_eq!(s.clip_of(ItemKind::Stroke, 6), Some(1));
    assert_eq!(s.clip_of(ItemKind::Stroke, 7), None, "an item no run names is not clipped");
    assert_eq!(s.clip_of(ItemKind::Fill, 2), Some(0));
    assert_eq!(s.chain_of(1), vec![0, 1]);
    assert_eq!(s.chain_of(2), vec![2]);
}

#[test]
fn counts_over_the_ceilings_are_comp_03_and_truncation_is_comp_01() {
    for (c, r) in [(65_537u32, 0u32), (0, (1 << 20) + 1)] {
        let mut h = c.to_le_bytes().to_vec();
        h.extend_from_slice(&r.to_le_bytes());
        assert_eq!(ClipStream::decode(&h).unwrap_err().rule(), "COMP-03", "{c} {r}");
    }
    let b = fixture().encode().unwrap();
    assert_eq!(ClipStream::decode(&b[..b.len() - 1]).unwrap_err().rule(), "COMP-01");
    let mut long = b;
    long.push(0);
    assert_eq!(ClipStream::decode(&long).unwrap_err().rule(), "COMP-02");
}

#[test]
fn a_boundary_of_no_entities_is_comp_05_and_over_4096_is_comp_03() {
    let s = ClipStream { clips: vec![clip(FillRule::NonZero, None, &[])], runs: vec![] };
    assert_eq!(rule_of(s), "COMP-05");
    let ring: Vec<u32> = (0..4097).collect();
    let s = ClipStream { clips: vec![clip(FillRule::NonZero, None, &ring)], runs: vec![] };
    assert_eq!(rule_of(s), "COMP-03");
    let s = ClipStream { clips: vec![clip(FillRule::NonZero, None, &ring[..4096])], runs: vec![] };
    assert!(s.encode().is_ok());
}

#[test]
fn a_boundary_kind_other_than_references_is_comp_10_and_reserved_bytes_are_comp_12() {
    let mut b = fixture().encode().unwrap();
    b[16..20].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(ClipStream::decode(&b).unwrap_err().rule(), "COMP-10");
    let mut b = fixture().encode().unwrap();
    b[9] = 1;
    assert_eq!(ClipStream::decode(&b).unwrap_err().rule(), "COMP-12");
    let mut b = fixture().encode().unwrap();
    b[8] = 2;
    assert_eq!(ClipStream::decode(&b).unwrap_err().rule(), "COMP-11", "a rule that is neither 0 nor 1");
}

#[test]
fn an_entity_id_at_or_above_the_limit_is_comp_08() {
    let mut b = fixture().encode().unwrap();
    b[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(ClipStream::decode(&b).unwrap_err().rule(), "COMP-08");
}

#[test]
fn a_parent_that_is_not_an_earlier_clip_is_comp_25() {
    for parent in [0u32, 1, 2, 7] {
        // clip 1 naming itself, a later clip, or a clip that does not exist as its parent
        let mut s = fixture();
        s.clips[1].parent = Some(parent + if parent == 0 { 1 } else { 0 });
        let want = if s.clips[1].parent.unwrap() as usize >= s.clips.len() { "COMP-25" } else { "COMP-25" };
        assert_eq!(rule_of(s), want, "parent {parent}");
    }
    // two clips naming each other
    let s = ClipStream {
        clips: vec![clip(FillRule::NonZero, Some(1), &[0]), clip(FillRule::NonZero, Some(0), &[1])],
        runs: vec![],
    };
    assert_eq!(rule_of(s), "COMP-25");
}

#[test]
fn a_chain_of_eight_is_allowed_and_nine_is_comp_26() {
    assert!(chain(8).encode().is_ok());
    assert_eq!(rule_of(chain(9)), "COMP-26");
    // the decoder holds hostile bytes to the same depth
    let mut s = chain(9);
    s.clips[8].parent = None;
    let mut b = s.encode().unwrap();
    // clip 8 starts after eight clips of 24 bytes each; give it the parent 7 by hand
    let at = 8 + 8 * 24 + 4;
    b[at..at + 4].copy_from_slice(&7u32.to_le_bytes());
    assert_eq!(ClipStream::decode(&b).unwrap_err().rule(), "COMP-26");
}

#[test]
fn a_run_names_a_clip_that_exists_or_it_is_comp_09() {
    let mut s = fixture();
    s.runs[0].clip = 3;
    assert_eq!(rule_of(s), "COMP-09");
}

#[test]
fn a_run_of_zero_items_is_comp_05_and_a_kind_past_4_is_comp_11() {
    let mut s = fixture();
    s.runs[0].count = 0;
    assert_eq!(rule_of(s), "COMP-05");
    let mut b = fixture().encode().unwrap();
    let at = b.len() - 16 * 4;
    b[at..at + 4].copy_from_slice(&5u32.to_le_bytes());
    assert_eq!(ClipStream::decode(&b).unwrap_err().rule(), "COMP-11");
}

#[test]
fn an_item_assigned_to_two_clips_is_comp_21_and_an_overflowing_run_follows_23() {
    let mut s = fixture();
    s.runs.push(run(ItemKind::Fill, 2, 2, 2));
    assert_eq!(rule_of(s), "COMP-21");
    let mut s = fixture();
    s.runs[1] = run(ItemKind::Stroke, u32::MAX, 2, 1);
    assert_eq!(rule_of(s), "COMP-08");
    let mut s = fixture();
    s.runs[0] = run(ItemKind::Fill, u32::MAX, 2, 0);
    assert_eq!(rule_of(s), "COMP-03");
}

#[test]
fn validation_checks_the_ring_entities_and_the_items() {
    let s = fixture();
    let all = |_: EntityId| true;
    assert_eq!(s.validate(all, all, 3, 1, 5), Ok(()));
    // a ring entity that is not in the geometry
    assert_eq!(s.validate(|e: EntityId| e.index() != 31, all, 3, 1, 5).unwrap_err().rule(), "COMP-22");
    // an item past the end of its stream
    assert_eq!(s.validate(all, all, 2, 1, 5).unwrap_err().rule(), "COMP-22");
    // a stroke that is not drawn
    assert_eq!(s.validate(all, |e: EntityId| e.index() != 6, 3, 1, 5).unwrap_err().rule(), "COMP-22");
    // a clip ring may be hidden: it is only exists, not drawn
    assert_eq!(s.validate(all, |e: EntityId| e.index() < 5, 3, 1, 5).unwrap_err().rule(), "COMP-22");
}

#[test]
fn it_is_framed_with_the_registry_version_and_an_older_reader_skips_it_by_length() {
    let r = Registry::v1();
    let framed = fixture().into_stream(&r).unwrap();
    assert_eq!((framed.capability, framed.version), (ids::CLIP, Version::new(1, 0, 0)));
    let set = encode_streams(&[framed]).unwrap();
    let knows = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::CLIP] };
    let opened = read_streams(&set, &r, &knows).unwrap();
    assert_eq!(ClipStream::decode(&opened.streams[0].payload).unwrap(), fixture());
    let old = Reader { version: Version::new(1, 0, 0), implemented: vec![ids::HATCH] };
    let opened = read_streams(&set, &r, &old).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped[0].reason, SkipReason::NotImplemented);
}
