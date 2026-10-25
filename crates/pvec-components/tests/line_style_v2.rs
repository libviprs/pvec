//! pvec#125: the line-style stream at version 2.0 carries a stroke width, a dash phase and a visibility
//! flag inline in every style. Layout per style, after the name and the dashes:
//! `u8 width_kind` (0 absent, 1 thinnest, 2 explicit), `f64 width` only for kind 2, `f64 phase`,
//! `u8 flags` (bit 0 visible, every other bit must be zero).
#![allow(missing_docs)]

use pvec_capabilities::{Class, Reader, Registry, Version, ids};
use pvec_components::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn def(name: &str, dashes: Vec<f64>, width: Option<StrokeWidth>, phase: f64, visible: bool) -> LineStyleDef {
    LineStyleDef { name: name.into(), dashes, width, phase, visible }
}

fn one(d: LineStyleDef) -> LineStyleStream {
    LineStyleStream { styles: vec![d], assignments: vec![(id(1), 0)] }
}

#[test]
fn issue_125_the_registry_declares_line_style_at_2_0_0() {
    let r = Registry::v1();
    assert_eq!(r.get(ids::LINE_STYLE).unwrap().version, Version::new(2, 0, 0));
    let s = one(def("D", vec![], None, 0.0, true)).into_stream(&r).unwrap();
    assert_eq!(s.version, Version::new(2, 0, 0));
}

#[test]
fn issue_125_the_bump_leaves_every_geometry_version_and_every_other_component_unchanged() {
    let r = Registry::v1();
    for c in r.entries().iter().filter(|c| c.class == Class::Geometry) {
        assert_eq!(c.version, Version::new(1, 0, 0), "{}", c.name);
    }
    for c in r.entries().iter().filter(|c| c.class == Class::Component && c.id != ids::LINE_STYLE) {
        assert_eq!(c.version.major, 1, "{} moved", c.name);
    }
    assert_eq!(r.get(ids::LINE_STYLE).unwrap().version.major, 2);
}

#[test]
fn issue_125_width_absent_thinnest_zero_and_explicit_round_trip_and_stay_distinct() {
    let cases =
        [None, Some(StrokeWidth::Thinnest), Some(StrokeWidth::Width(0.0)), Some(StrokeWidth::Width(0.25))];
    let mut encodings = Vec::new();
    for w in cases {
        let s = one(def("W", vec![], w, 0.0, true));
        let b = s.encode().unwrap();
        assert_eq!(LineStyleStream::decode(&b).unwrap(), s, "{w:?}");
        encodings.push(b);
    }
    for i in 0..encodings.len() {
        for j in i + 1..encodings.len() {
            assert_ne!(encodings[i], encodings[j], "{:?} and {:?} encode the same", cases[i], cases[j]);
        }
    }
}

#[test]
fn issue_125_the_width_is_written_only_for_an_explicit_width() {
    let len = |w| one(def("W", vec![], w, 0.0, true)).encode().unwrap().len();
    let absent = len(None);
    assert_eq!(len(Some(StrokeWidth::Thinnest)), absent, "thinnest carries no f64");
    assert_eq!(len(Some(StrokeWidth::Width(1.0))), absent + 8);
}

#[test]
fn issue_125_exact_byte_layout_of_one_style() {
    let b = one(def("A", vec![2.0], Some(StrokeWidth::Width(0.5)), 1.5, false)).encode().unwrap();
    // the style starts after the two u32 counts
    let mut e = Vec::new();
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&1u32.to_le_bytes());
    e.push(b'A');
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&2.0f64.to_le_bytes());
    e.push(2);
    e.extend_from_slice(&0.5f64.to_le_bytes());
    e.extend_from_slice(&1.5f64.to_le_bytes());
    e.push(0);
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(b, e);
}

#[test]
fn issue_125_phase_and_visibility_round_trip() {
    for (phase, visible) in [(0.0, true), (0.0, false), (3.5, true), (3.5, false)] {
        let s = one(def("P", vec![4.0, 2.0], Some(StrokeWidth::Thinnest), phase, visible));
        assert_eq!(LineStyleStream::decode(&s.encode().unwrap()).unwrap(), s);
    }
}

#[test]
fn issue_125_the_writer_refuses_bad_widths_phases_and_a_phase_without_dashes() {
    for w in [f64::NAN, f64::INFINITY, -0.5] {
        let s = one(def("W", vec![], Some(StrokeWidth::Width(w)), 0.0, true));
        assert_eq!(s.encode().unwrap_err(), ComponentError::BadNumber, "width {w}");
    }
    for p in [f64::NAN, f64::NEG_INFINITY] {
        let s = one(def("P", vec![1.0], None, p, true));
        assert_eq!(s.encode().unwrap_err(), ComponentError::BadNumber, "phase {p}");
    }
    let s = one(def("P", vec![], None, 2.0, true));
    assert_eq!(s.encode().unwrap_err(), ComponentError::BadNumber, "phase with no dashes");
}

#[test]
fn issue_125_the_reader_refuses_the_same_things_on_bytes_the_writer_did_not_write() {
    let good = one(def("A", vec![2.0], Some(StrokeWidth::Width(0.5)), 1.5, true)).encode().unwrap();
    let n = good.len();
    // trailing layout: ... kind(1) width(8) phase(8) flags(1) then the 8 byte assignment
    let flags_at = n - 8 - 1;
    let phase_at = flags_at - 8;
    let width_at = phase_at - 8;
    let kind_at = width_at - 1;
    for bad in [f64::NAN, f64::INFINITY, -1.0] {
        let mut b = good.clone();
        b[width_at..width_at + 8].copy_from_slice(&bad.to_le_bytes());
        assert_eq!(LineStyleStream::decode(&b).unwrap_err(), ComponentError::BadNumber, "width {bad}");
    }
    for bad in [f64::NAN, f64::INFINITY] {
        let mut b = good.clone();
        b[phase_at..phase_at + 8].copy_from_slice(&bad.to_le_bytes());
        assert_eq!(LineStyleStream::decode(&b).unwrap_err(), ComponentError::BadNumber, "phase {bad}");
    }
    for flags in [2u8, 0x80, 3] {
        let mut b = good.clone();
        b[flags_at] = flags;
        assert_eq!(LineStyleStream::decode(&b).unwrap_err(), ComponentError::ReservedNotZero, "{flags}");
    }
    for kind in [3u8, 255] {
        let mut b = good.clone();
        b[kind_at] = kind;
        assert_eq!(LineStyleStream::decode(&b).unwrap_err(), ComponentError::BadTag(kind as u32), "{kind}");
    }
    // a phase on a style without dashes
    let flat = one(def("F", vec![], None, 0.0, true)).encode().unwrap();
    let fphase_at = flat.len() - 8 - 1 - 8;
    let mut b = flat.clone();
    b[fphase_at..fphase_at + 8].copy_from_slice(&1.0f64.to_le_bytes());
    assert_eq!(LineStyleStream::decode(&b).unwrap_err(), ComponentError::BadNumber);
}

#[test]
fn issue_125_every_truncation_is_refused() {
    let b = LineStyleStream {
        styles: vec![
            def("A", vec![1.0, 2.0], Some(StrokeWidth::Width(0.5)), 0.5, true),
            def("B", vec![], Some(StrokeWidth::Thinnest), 0.0, false),
            def("C", vec![], None, 0.0, true),
        ],
        assignments: vec![(id(0), 2), (id(1), 0)],
    }
    .encode()
    .unwrap();
    assert!(LineStyleStream::decode(&b).is_ok());
    for cut in 0..b.len() {
        assert!(LineStyleStream::decode(&b[..cut]).is_err(), "cut at {cut}");
    }
    let mut longer = b.clone();
    longer.push(0);
    assert_eq!(LineStyleStream::decode(&longer).unwrap_err(), ComponentError::Trailing);
}

#[test]
fn issue_125_a_1_0_stream_is_skipped_by_version_and_a_2_0_stream_is_read() {
    let r = Registry::v1();
    let reader = Reader { version: Version::new(1, 0, 0), implemented: vec![ids::LINE_STYLE] };
    let s = one(def("D", vec![1.0], None, 0.0, true)).into_stream(&r).unwrap();
    let old = Stream { version: Version::new(1, 0, 0), ..s.clone() };
    let bytes = encode_streams(&[old, s.clone()]).unwrap_or_else(|e| panic!("{e:?}"));
    let opened = read_streams(&bytes, &r, &reader).unwrap();
    assert_eq!(opened.streams.len(), 1);
    assert_eq!(opened.streams[0].version, Version::new(2, 0, 0));
    assert_eq!(opened.skipped.len(), 1);
    assert_eq!(opened.skipped[0].reason, SkipReason::IncompatibleVersion);
}
