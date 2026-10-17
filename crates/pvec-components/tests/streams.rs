//! Independent component streams (P3.7): framing that makes an unknown stream skippable without parsing
//! its payload, the typed payloads that refer to entities by `EntityId`, and the independence of all of
//! it from the geometry.
#![allow(missing_docs)]

use pvec_capabilities::{CapabilityId, Class, Declaration, Declared, Reader, Registry, Requirement, Version, ids};
use pvec_components::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn reg() -> Registry {
    Registry::v1()
}

fn reader_knowing(all: &[CapabilityId]) -> Reader {
    Reader { version: Version::new(1, 0, 0), implemented: all.to_vec() }
}

const ALL: [CapabilityId; 6] =
    [ids::LAYER, ids::LINE_STYLE, ids::TEXT, ids::HATCH, ids::SOURCE_PROVENANCE, ids::DWG_SEMANTICS];

// ---------------------------------------------------------------- the registry side

#[test]
fn the_six_component_streams_are_registered_optional_with_their_own_ids_and_versions() {
    let r = reg();
    for (cap, name, n) in [
        (ids::LAYER, "layer", 14),
        (ids::LINE_STYLE, "line_style", 15),
        (ids::TEXT, "text", 16),
        (ids::HATCH, "hatch", 17),
        (ids::SOURCE_PROVENANCE, "source_provenance", 18),
        (ids::DWG_SEMANTICS, "dwg_semantics", 19),
    ] {
        let c = r.get(cap).unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!((c.name, c.id.get()), (name, n));
        assert_eq!(c.class, Class::Component, "{name}");
        assert_eq!(c.requirement, Requirement::Optional, "{name} is optional by construction");
        assert!(c.iso_construct.is_empty(), "{name} names no ISO construct");
        assert_eq!(c.version, Version::new(1, 0, 0), "{name}");
    }
}

#[test]
fn no_component_stream_declares_a_geometry_capability_id() {
    for c in reg().entries().iter().filter(|c| c.class == Class::Component) {
        let geometry_ids: Vec<u32> =
            reg().entries().iter().filter(|g| g.class == Class::Geometry).map(|g| g.id.get()).collect();
        assert!(!geometry_ids.contains(&c.id.get()), "{} reuses a geometry id", c.name);
    }
    // and the stream reader rejects a stream that carries one
    let bad = encode_streams(&[Stream { capability: ids::LINE, version: Version::new(1, 0, 0), payload: vec![] }]);
    assert_eq!(bad.unwrap_err(), StreamError::GeometryCapability(ids::LINE));
    // a hand-built frame with a geometry id is refused on read as well
    let mut b = encode_streams(&[Stream { capability: ids::LAYER, version: Version::new(1, 0, 0), payload: vec![] }]).unwrap();
    b[SET_HEADER..SET_HEADER + 4].copy_from_slice(&ids::CIRCLE.get().to_le_bytes());
    assert_eq!(read_streams(&b, &reg(), &reader_knowing(&ALL)).unwrap_err(), StreamError::GeometryCapability(ids::CIRCLE));
}

#[test]
fn bumping_a_component_streams_version_leaves_every_geometry_capability_version_unchanged() {
    let before = reg();
    let mut after = reg();
    after.bump_component_version(ids::LAYER, Version::new(1, 1, 0)).unwrap();
    assert_eq!(after.get(ids::LAYER).unwrap().version, Version::new(1, 1, 0));
    for (a, b) in before.entries().iter().zip(after.entries()) {
        if a.id != ids::LAYER {
            assert_eq!(a, b, "{} changed", a.name);
        }
    }
    for c in after.entries().iter().filter(|c| c.class == Class::Geometry) {
        assert_eq!(c.version, Version::new(1, 0, 0), "{}", c.name);
    }
    // only a component can be bumped this way, and only upwards
    assert!(after.bump_component_version(ids::LINE, Version::new(1, 1, 0)).is_err());
    assert!(after.bump_component_version(ids::LAYER, Version::new(1, 0, 5)).is_err());
    assert!(after.bump_component_version(ids::LAYER, Version::new(1, 1, 0)).is_err(), "equal is not a bump");
    assert!(after.bump_component_version(CapabilityId::new(9999), Version::new(2, 0, 0)).is_err());
}

// ---------------------------------------------------------------- framing

fn stream(cap: CapabilityId, payload: &[u8]) -> Stream {
    Stream { capability: cap, version: Version::new(1, 0, 0), payload: payload.to_vec() }
}

/// Bytes before the first frame, from docs/COMPONENT_STREAMS.md.
const SET_HEADER: usize = 16;

#[test]
fn streams_round_trip_through_the_framing() {
    let streams = vec![stream(ids::LAYER, b"abc"), stream(ids::TEXT, &[]), stream(ids::HATCH, &[7; 100])];
    let b = encode_streams(&streams).unwrap();
    assert_eq!(&b[..4], b"PVCS");
    assert_eq!(b.len() % 8, 0);
    let opened = read_streams(&b, &reg(), &reader_knowing(&ALL)).unwrap();
    assert_eq!(opened.streams, streams);
    assert!(opened.skipped.is_empty());
    assert_eq!(read_streams(&encode_streams(&[]).unwrap(), &reg(), &reader_knowing(&ALL)).unwrap().streams.len(), 0);
}

#[test]
fn an_unknown_stream_is_reported_skipped_without_parsing_its_payload_and_is_not_an_error() {
    // The unknown stream's payload is garbage on purpose: nothing may read it.
    let alien = CapabilityId::new(40_000);
    let streams = vec![stream(ids::LAYER, b"ok"), stream(alien, &[0xff; 37]), stream(ids::TEXT, b"fine")];
    let b = encode_streams(&streams);
    let opened = read_streams(&b, &reg(), &reader_knowing(&ALL)).unwrap();
    assert_eq!(opened.streams.len(), 2);
    assert_eq!(opened.skipped.len(), 1);
    assert_eq!(opened.skipped[0].capability, alien);
    assert_eq!(opened.skipped[0].reason, SkipReason::UnknownCapability);
    assert_eq!(opened.skipped[0].version, Version::new(1, 0, 0));
}

#[test]
fn a_known_stream_the_reader_does_not_implement_is_skipped_too() {
    let b = encode_streams(&[stream(ids::HATCH, b"payload")]).unwrap();
    let opened = read_streams(&b, &reg(), &reader_knowing(&[ids::LAYER])).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped[0].reason, SkipReason::NotImplemented);
}

#[test]
fn a_stream_from_a_newer_major_version_is_skipped_not_misread() {
    let mut s = stream(ids::LAYER, b"x");
    s.version = Version::new(2, 0, 0);
    let b = encode_streams(&[s]).unwrap();
    let opened = read_streams(&b, &reg(), &reader_knowing(&ALL)).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped[0].reason, SkipReason::IncompatibleVersion);
    // a newer minor of the same major is read
    let mut s = stream(ids::LAYER, b"x");
    s.version = Version::new(1, 4, 0);
    let opened = read_streams(&encode_streams(&[s]).unwrap(), &reg(), &reader_knowing(&ALL)).unwrap();
    assert_eq!(opened.streams.len(), 1);
}

#[test]
fn a_reader_implementing_none_of_the_streams_skips_them_all_and_negotiation_still_accepts_the_file() {
    let streams: Vec<Stream> = ALL.iter().map(|c| stream(*c, b"anything")).collect();
    let b = encode_streams(&streams).unwrap();
    let nobody = reader_knowing(&[]);
    let opened = read_streams(&b, &reg(), &nobody).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped.len(), 6);
    let file = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: ALL.iter().map(|c| Declared { id: *c, requirement: Requirement::Optional, version: Version::new(1, 0, 0) }).collect(),
    };
    let n = reg().negotiate(&nobody, &file).unwrap();
    assert_eq!(n.skipped.len(), 6);
    // declaring a component required is refused by the registry
    let mut required = file.clone();
    required.capabilities[0].requirement = Requirement::Required;
    assert!(reg().negotiate(&nobody, &required).is_err());
}

#[test]
fn the_framing_refuses_every_truncation_and_every_lie() {
    let b = encode_streams(&[stream(ids::LAYER, b"hello"), stream(ids::TEXT, b"world!!")]).unwrap();
    for n in 0..b.len() {
        assert!(read_streams(&b[..n], &reg(), &reader_knowing(&ALL)).is_err(), "cut at {n}");
    }
    let mut c = b.clone();
    c[0] ^= 1;
    assert_eq!(read_streams(&c, &reg(), &reader_knowing(&ALL)).unwrap_err(), StreamError::BadMagic);
    let mut c = b.clone();
    c[4] = 9;
    assert!(matches!(read_streams(&c, &reg(), &reader_knowing(&ALL)).unwrap_err(), StreamError::UnsupportedVersion(_)));
    let mut c = b.clone();
    c[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(read_streams(&c, &reg(), &reader_knowing(&ALL)).unwrap_err(), StreamError::Truncated | StreamError::TooMany));
    // a payload length past the end of the buffer
    let mut c = b.clone();
    let len_at = SET_HEADER + 12;
    c[len_at..len_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(read_streams(&c, &reg(), &reader_knowing(&ALL)).unwrap_err(), StreamError::Truncated);
    // trailing bytes
    let mut c = b.clone();
    c.extend_from_slice(&[0; 8]);
    assert!(read_streams(&c, &reg(), &reader_knowing(&ALL)).is_err());
    // non-zero padding after a 5 byte payload
    let mut c = b.clone();
    c[SET_HEADER + 16 + 5] = 1;
    assert_eq!(read_streams(&c, &reg(), &reader_knowing(&ALL)).unwrap_err(), StreamError::NonZeroPadding);
}

#[test]
fn random_corruption_of_a_stream_set_never_panics() {
    let streams: Vec<Stream> = ALL.iter().map(|c| stream(*c, &[1, 2, 3, 4, 5])).collect();
    let good = encode_streams(&streams).unwrap();
    let mut x: u64 = 0x1234_5678_9abc_def1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..3000 {
        let mut b = good.clone();
        for _ in 0..1 + (next() % 4) as usize {
            let at = (next() % b.len() as u64) as usize;
            b[at] = next() as u8;
        }
        let _ = read_streams(&b, &reg(), &reader_knowing(&ALL));
    }
}

#[test]
fn the_same_streams_always_encode_to_the_same_bytes() {
    let streams = vec![stream(ids::LAYER, b"a"), stream(ids::HATCH, b"bb")];
    assert_eq!(encode_streams(&streams).unwrap(), encode_streams(&streams).unwrap());
    assert!(EntityId::new(0).is_ok());
    let _ = id(0);
}
