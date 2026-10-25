//! Issue #119: the layer stream carries the on, frozen and locked state of each layer (stream 2.0).
#![allow(missing_docs)]

use pvec_capabilities::{Class, Registry, Version, ids};
use pvec_components::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn stream(flags: Vec<u8>) -> LayerStream {
    let names = ["SHOWN_L", "OFF_L", "FROZEN_L", "LOCKED_L"].iter().take(flags.len()).map(|n| n.to_string());
    LayerStream { names: names.collect(), flags, assignments: vec![(id(0), 0)] }
}

#[test]
fn issue_119_flag_bits_are_off_frozen_locked() {
    assert_eq!((LAYER_OFF, LAYER_FROZEN, LAYER_LOCKED), (1, 2, 4));
}

#[test]
fn issue_119_flags_round_trip_for_every_allowed_combination() {
    let s = stream(vec![0, LAYER_OFF, LAYER_FROZEN, LAYER_LOCKED]);
    assert_eq!(LayerStream::decode(&s.encode().unwrap()).unwrap(), s);
    let all: Vec<u8> = (0..8).collect();
    let names = (0..8).map(|i| format!("L{i}")).collect();
    let s = LayerStream { names, flags: all, assignments: vec![] };
    let back = LayerStream::decode(&s.encode().unwrap()).unwrap();
    assert_eq!(back, s);
}

#[test]
fn issue_119_the_flags_byte_follows_each_name_in_the_table() {
    let s =
        LayerStream { names: vec!["A".into(), "B".into()], flags: vec![5, 2], assignments: vec![(id(3), 1)] };
    let mut want = Vec::new();
    want.extend_from_slice(&2u32.to_le_bytes()); // names
    want.extend_from_slice(&1u32.to_le_bytes()); // assignments
    want.extend_from_slice(&[1, 0, 0, 0, b'A', 5]);
    want.extend_from_slice(&[1, 0, 0, 0, b'B', 2]);
    want.extend_from_slice(&3u32.to_le_bytes());
    want.extend_from_slice(&1u32.to_le_bytes());
    assert_eq!(s.encode().unwrap(), want);
}

#[test]
fn issue_119_a_reserved_bit_is_refused_on_write_and_on_read() {
    for bit in 3..8 {
        let bad = stream(vec![1 << bit]);
        assert_eq!(bad.encode().unwrap_err(), ComponentError::ReservedNotZero, "bit {bit} on write");
        let mut b = stream(vec![0]).encode().unwrap();
        let at = 8 + 4 + "SHOWN_L".len();
        b[at] = 1 << bit;
        assert_eq!(
            LayerStream::decode(&b).unwrap_err(),
            ComponentError::ReservedNotZero,
            "bit {bit} on read"
        );
    }
    assert_eq!(ComponentError::ReservedNotZero.rule(), "COMP-12");
}

#[test]
fn issue_119_one_flags_byte_per_name_or_the_stream_is_refused() {
    let s = LayerStream { names: vec!["A".into(), "B".into()], flags: vec![0], assignments: vec![] };
    assert!(s.encode().is_err());
    let s = LayerStream { names: vec!["A".into()], flags: vec![0, 0], assignments: vec![] };
    assert!(s.encode().is_err());
}

#[test]
fn issue_119_a_payload_that_ends_before_a_flags_byte_is_truncated() {
    let mut b = Vec::new();
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&[1, 0, 0, 0, b'A']); // the name, and no flags byte
    assert_eq!(LayerStream::decode(&b).unwrap_err(), ComponentError::Truncated);
}

#[test]
fn issue_119_a_1_0_payload_decodes_with_every_flag_zero() {
    let mut v1 = Vec::new();
    v1.extend_from_slice(&2u32.to_le_bytes());
    v1.extend_from_slice(&1u32.to_le_bytes());
    v1.extend_from_slice(&[1, 0, 0, 0, b'A']);
    v1.extend_from_slice(&[1, 0, 0, 0, b'B']);
    v1.extend_from_slice(&0u32.to_le_bytes());
    v1.extend_from_slice(&1u32.to_le_bytes());
    let s = LayerStream::decode_at(Version::new(1, 0, 0), &v1).unwrap();
    assert_eq!(s.names, ["A", "B"]);
    assert_eq!(s.flags, [0, 0]);
    assert_eq!(s.assignments, [(id(0), 1)]);
    // the same bytes are not a 2.0 payload
    assert!(LayerStream::decode_at(Version::new(2, 0, 0), &v1).is_err());
    // and a major this decoder has no layout for is refused, not guessed at
    assert!(LayerStream::decode_at(Version::new(3, 0, 0), &v1).is_err());
}

#[test]
fn issue_119_the_registry_gives_the_layer_stream_version_2_and_nothing_else_moves() {
    let r = Registry::v1();
    assert_eq!(r.get(ids::LAYER).unwrap().version, Version::new(2, 0, 0));
    for c in r.entries() {
        if c.id != ids::LAYER {
            assert_eq!(c.version, Version::new(1, 0, 0), "{}", c.name);
        }
        if c.class == Class::Geometry {
            assert_eq!(c.version, Version::new(1, 0, 0), "{}", c.name);
        }
    }
    let s = stream(vec![LAYER_OFF]).into_stream(&r).unwrap();
    assert_eq!(s.version, Version::new(2, 0, 0));
}
