//! Capability 24, `stroke_paint` (issue 286, `docs/PAINT.md` section 2.3): a colour table and pairs of an
//! entity and a colour index, in the shape of the line style stream. Written before the codec.
#![allow(missing_docs)]

use pvec_capabilities::{Class, Reader, Registry, Requirement, Version, ids};
use pvec_components::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn fixture() -> StrokePaintStream {
    StrokePaintStream {
        colours: vec![0xFF00_00FF, 0x1020_3040, 0x0000_0080],
        assignments: vec![(id(0), 0), (id(3), 2), (id(4), 2), (id(900), 1)],
    }
}

/// A payload by hand: colour count, assignment count, the words, the pairs.
fn raw(colours: &[u32], pairs: &[(u32, u32)]) -> Vec<u8> {
    let mut b = (colours.len() as u32).to_le_bytes().to_vec();
    b.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
    for c in colours {
        b.extend_from_slice(&c.to_le_bytes());
    }
    for (e, i) in pairs {
        b.extend_from_slice(&e.to_le_bytes());
        b.extend_from_slice(&i.to_le_bytes());
    }
    b
}

fn rule_of_raw(colours: &[u32], pairs: &[(u32, u32)]) -> &'static str {
    StrokePaintStream::decode(&raw(colours, pairs)).expect_err("refused").rule()
}

#[test]
fn the_registry_has_capability_24_as_an_optional_component_at_1_0_0() {
    let r = Registry::v1();
    let c = r.get(ids::STROKE_PAINT).expect("24 is registered");
    assert_eq!((c.id.get(), c.name), (24, "stroke_paint"));
    assert_eq!(c.class, Class::Component);
    assert_eq!(c.requirement, Requirement::Optional);
    assert_eq!(c.version, Version::new(1, 0, 0));
    assert_eq!(c.min_reader, Version::new(1, 0, 0));
    assert!(c.iso_construct.is_empty());
}

#[test]
fn the_layout_is_two_counts_the_colour_words_then_entity_and_index_pairs() {
    let s = fixture();
    let bytes = s.encode().unwrap();
    assert_eq!(bytes, raw(&[0xFF00_00FF, 0x1020_3040, 0x0000_0080], &[(0, 0), (3, 2), (4, 2), (900, 1)]));
    assert_eq!(bytes.len(), 8 + 4 * 3 + 8 * 4);
}

#[test]
fn it_round_trips_with_the_rgba_word_untouched() {
    let s = fixture();
    let back = StrokePaintStream::decode(&s.encode().unwrap()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.colours[0], 0xFF00_00FF, "0xRRGGBBAA, straight alpha");
}

#[test]
fn a_table_with_no_assignment_is_valid_and_says_nothing_about_any_entity() {
    let s = StrokePaintStream { colours: vec![0x0000_00FF], assignments: vec![] };
    assert_eq!(StrokePaintStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn colour_lookup_gives_the_colour_of_an_entity_or_nothing() {
    let s = fixture();
    assert_eq!(s.colour_of(id(3)), Some(0x0000_0080));
    assert_eq!(s.colour_of(id(900)), Some(0x1020_3040));
    assert_eq!(s.colour_of(id(1)), None, "no assignment means the viewer's own style applies");
}

#[test]
fn a_payload_that_ends_early_is_comp_01_and_one_with_extra_bytes_is_comp_02() {
    let mut b = fixture().encode().unwrap();
    b.pop();
    assert_eq!(StrokePaintStream::decode(&b).unwrap_err().rule(), "COMP-01");
    let mut b = fixture().encode().unwrap();
    b.push(0);
    assert_eq!(StrokePaintStream::decode(&b).unwrap_err().rule(), "COMP-02");
    assert_eq!(StrokePaintStream::decode(&[1, 0, 0]).unwrap_err().rule(), "COMP-01");
}

#[test]
fn more_than_65536_colours_or_2_pow_24_assignments_are_comp_03() {
    let mut b = 65_537u32.to_le_bytes().to_vec();
    b.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(StrokePaintStream::decode(&b).unwrap_err().rule(), "COMP-03");
    let mut b = 1u32.to_le_bytes().to_vec();
    b.extend_from_slice(&((1u32 << 24) + 1).to_le_bytes());
    assert_eq!(StrokePaintStream::decode(&b).unwrap_err().rule(), "COMP-03");
    let s = StrokePaintStream { colours: vec![0; 65_537], assignments: vec![] };
    assert_eq!(s.encode().unwrap_err().rule(), "COMP-03");
}

#[test]
fn no_colours_at_all_is_comp_05() {
    assert_eq!(rule_of_raw(&[], &[]), "COMP-05");
    assert_eq!(
        StrokePaintStream { colours: vec![], assignments: vec![] }.encode().unwrap_err().rule(),
        "COMP-05"
    );
}

#[test]
fn the_most_colours_allowed_round_trip() {
    let s = StrokePaintStream { colours: vec![0xAABB_CCDD; 65_536], assignments: vec![(id(0), 65_535)] };
    assert_eq!(StrokePaintStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn a_colour_index_with_no_entry_is_comp_09() {
    assert_eq!(rule_of_raw(&[1, 2], &[(0, 2)]), "COMP-09");
    let s = StrokePaintStream { colours: vec![1], assignments: vec![(id(0), 1)] };
    assert_eq!(s.encode().unwrap_err().rule(), "COMP-09");
}

#[test]
fn an_entity_id_at_or_above_the_limit_is_comp_08() {
    assert_eq!(rule_of_raw(&[1], &[(u32::MAX, 0)]), "COMP-08");
}

#[test]
fn entities_that_do_not_strictly_ascend_are_comp_28() {
    assert_eq!(rule_of_raw(&[1], &[(5, 0), (5, 0)]), "COMP-28", "equal");
    assert_eq!(rule_of_raw(&[1], &[(5, 0), (2, 0)]), "COMP-28", "descending");
    let s = StrokePaintStream { colours: vec![1], assignments: vec![(id(5), 0), (id(2), 0)] };
    assert_eq!(s.encode().unwrap_err().rule(), "COMP-28");
}

#[test]
fn the_stream_is_framed_and_a_reader_without_24_skips_it_by_length() {
    let r = Registry::v1();
    let framed = fixture().into_stream(&r).unwrap();
    assert_eq!((framed.capability, framed.version), (ids::STROKE_PAINT, Version::new(1, 0, 0)));
    let set = encode_streams(&[framed]).unwrap();
    let knows = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::STROKE_PAINT] };
    let opened = read_streams(&set, &r, &knows).unwrap();
    assert_eq!(StrokePaintStream::decode(&opened.streams[0].payload).unwrap(), fixture());
    let old = Reader { version: Version::new(1, 0, 0), implemented: vec![ids::LINE_STYLE] };
    let opened = read_streams(&set, &r, &old).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped[0].reason, SkipReason::NotImplemented);
}

#[test]
fn text_takes_its_colour_from_its_anchor_entity_in_this_same_table() {
    // docs/PAINT.md 2.3: no colour field in the text stream; the anchor's assignment is the text colour
    let s = fixture();
    let text = TextRecord { entity: id(4), x: 0.0, y: 0.0, height: 1.0, rotation: 0.0, text: "A".into() };
    assert_eq!(s.colour_of(text.entity), Some(0x0000_0080));
}
