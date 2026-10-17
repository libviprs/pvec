//! The typed payloads (P3.7): every stream refers to entities by `EntityId` and never embeds geometry.
#![allow(missing_docs)]

use pvec_components::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

#[test]
fn layer_streams_round_trip_and_check_their_indices() {
    let s = LayerStream {
        names: vec!["WALLS".into(), "Dimensions".into(), "caf\u{e9}".into()],
        assignments: vec![(id(0), 0), (id(5), 2), (id(9), 1)],
    };
    assert_eq!(LayerStream::decode(&s.encode().unwrap()).unwrap(), s);
    let bad = LayerStream { names: vec!["A".into()], assignments: vec![(id(0), 1)] };
    assert_eq!(bad.encode().unwrap_err(), ComponentError::IndexOutOfRange);
    // the decoder checks the same thing on bytes the encoder did not write
    let mut b = s.encode().unwrap();
    let n = b.len();
    b[n - 4..].copy_from_slice(&99u32.to_le_bytes());
    assert_eq!(LayerStream::decode(&b).unwrap_err(), ComponentError::IndexOutOfRange);
}

#[test]
fn line_style_streams_round_trip_and_refuse_bad_dashes() {
    let s = LineStyleStream {
        styles: vec![
            LineStyleDef { name: "CONTINUOUS".into(), dashes: vec![] },
            LineStyleDef { name: "DASHED".into(), dashes: vec![12.0, 6.0] },
            LineStyleDef { name: "DOTDASH".into(), dashes: vec![0.5, 3.0, 12.0, 3.0] },
        ],
        assignments: vec![(id(1), 1), (id(2), 2)],
    };
    assert_eq!(LineStyleStream::decode(&s.encode().unwrap()).unwrap(), s);
    for bad in [f64::NAN, f64::INFINITY, -1.0] {
        let b = LineStyleStream { styles: vec![LineStyleDef { name: "X".into(), dashes: vec![bad, 1.0] }], assignments: vec![] };
        assert!(b.encode().is_err(), "{bad}");
    }
    let long = LineStyleStream { styles: vec![LineStyleDef { name: "X".into(), dashes: vec![1.0; 17] }], assignments: vec![] };
    assert_eq!(long.encode().unwrap_err(), ComponentError::TooMany);
}

#[test]
fn text_streams_round_trip_and_refuse_bad_numbers_and_oversized_text() {
    let s = TextStream {
        records: vec![
            TextRecord { entity: id(3), x: 12.5, y: -7.25, height: 2.5, rotation: 0.5, text: "Room 101".into() },
            TextRecord { entity: id(4), x: 0.0, y: 0.0, height: 1.0, rotation: 0.0, text: String::new() },
        ],
    };
    assert_eq!(TextStream::decode(&s.encode().unwrap()).unwrap(), s);
    let mut bad = s.clone();
    bad.records[0].height = 0.0;
    assert_eq!(bad.encode().unwrap_err(), ComponentError::BadNumber);
    let mut bad = s.clone();
    bad.records[0].x = f64::NAN;
    assert_eq!(bad.encode().unwrap_err(), ComponentError::BadNumber);
    let mut bad = s.clone();
    bad.records[0].text = "x".repeat(4097);
    assert_eq!(bad.encode().unwrap_err(), ComponentError::TooLong);
}

#[test]
fn a_hatch_is_boundary_references_plus_fill_parameters_and_embeds_no_curve() {
    let s = HatchStream {
        records: vec![
            HatchRecord { boundary: vec![id(1), id(2), id(3)], fill: Fill::Solid { rgba: 0xff00_00ff } },
            HatchRecord {
                boundary: vec![id(7)],
                fill: Fill::Pattern { name: "ANSI31".into(), angle: 0.785_398, scale: 2.0, rgba: 0x0000_00ff },
            },
        ],
    };
    assert_eq!(HatchStream::decode(&s.encode().unwrap()).unwrap(), s);

    // Embedding curve parameters in the stream: the boundary tag says "inline curve" (2) instead of
    // "references" (1), and the decoder refuses it as embedded geometry.
    let mut b = s.encode().unwrap();
    let tag_at = hatch_first_boundary_tag_offset();
    assert_eq!(u32::from_le_bytes(b[tag_at..tag_at + 4].try_into().unwrap()), 1, "references");
    b[tag_at..tag_at + 4].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(HatchStream::decode(&b).unwrap_err(), ComponentError::EmbeddedGeometry);

    // an empty boundary and an absurd one
    let empty = HatchStream { records: vec![HatchRecord { boundary: vec![], fill: Fill::Solid { rgba: 0 } }] };
    assert_eq!(empty.encode().unwrap_err(), ComponentError::BadCount);
    let huge = HatchStream { records: vec![HatchRecord { boundary: vec![id(0); 4097], fill: Fill::Solid { rgba: 0 } }] };
    assert_eq!(huge.encode().unwrap_err(), ComponentError::TooMany);
    let bad_scale = HatchStream {
        records: vec![HatchRecord { boundary: vec![id(0)], fill: Fill::Pattern { name: "A".into(), angle: 0.0, scale: 0.0, rgba: 0 } }],
    };
    assert_eq!(bad_scale.encode().unwrap_err(), ComponentError::BadNumber);
}

/// Offset of the boundary-kind word of the first record: record count (u32), then the tag.
fn hatch_first_boundary_tag_offset() -> usize {
    4
}

#[test]
fn provenance_keeps_the_source_identity_outside_the_entity_id() {
    let s = ProvenanceStream {
        records: vec![
            ProvenanceRecord { entity: id(0), source_kind: 1, handle: b"2A7F".to_vec() },
            ProvenanceRecord { entity: id(1), source_kind: 2, handle: vec![0, 1, 2, 255] },
        ],
    };
    assert_eq!(ProvenanceStream::decode(&s.encode().unwrap()).unwrap(), s);
    let long = ProvenanceStream { records: vec![ProvenanceRecord { entity: id(0), source_kind: 1, handle: vec![1; 65] }] };
    assert_eq!(long.encode().unwrap_err(), ComponentError::TooLong);
}

#[test]
fn the_reserved_dwg_semantics_stream_is_opaque() {
    let s = ReservedStream { payload: vec![1, 2, 3, 4, 5] };
    assert_eq!(ReservedStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn typed_streams_know_their_registry_capability_and_take_their_version_from_it() {
    use pvec_capabilities::{Registry, ids};
    assert_eq!(LayerStream::CAPABILITY, ids::LAYER);
    assert_eq!(LineStyleStream::CAPABILITY, ids::LINE_STYLE);
    assert_eq!(TextStream::CAPABILITY, ids::TEXT);
    assert_eq!(HatchStream::CAPABILITY, ids::HATCH);
    assert_eq!(ProvenanceStream::CAPABILITY, ids::SOURCE_PROVENANCE);
    assert_eq!(ReservedStream::CAPABILITY, ids::DWG_SEMANTICS);
    let r = Registry::v1();
    let s = LayerStream { names: vec!["A".into()], assignments: vec![(id(0), 0)] }.into_stream(&r).unwrap();
    assert_eq!(s.capability, ids::LAYER);
    assert_eq!(s.version, r.get(ids::LAYER).unwrap().version);
}

// ---------------------------------------------------------------- hostile payloads

#[test]
fn every_truncation_of_every_payload_is_refused_and_counts_cannot_size_an_allocation() {
    let payloads: Vec<(&str, Vec<u8>, fn(&[u8]) -> bool)> = vec![
        ("layer", LayerStream { names: vec!["A".into(), "B".into()], assignments: vec![(id(0), 1)] }.encode().unwrap(), |b| LayerStream::decode(b).is_ok()),
        ("line_style", LineStyleStream { styles: vec![LineStyleDef { name: "D".into(), dashes: vec![1.0, 2.0] }], assignments: vec![(id(0), 0)] }.encode().unwrap(), |b| LineStyleStream::decode(b).is_ok()),
        ("text", TextStream { records: vec![TextRecord { entity: id(0), x: 1.0, y: 2.0, height: 3.0, rotation: 0.0, text: "hi".into() }] }.encode().unwrap(), |b| TextStream::decode(b).is_ok()),
        ("hatch", HatchStream { records: vec![HatchRecord { boundary: vec![id(0), id(1)], fill: Fill::Solid { rgba: 1 } }] }.encode().unwrap(), |b| HatchStream::decode(b).is_ok()),
        ("provenance", ProvenanceStream { records: vec![ProvenanceRecord { entity: id(0), source_kind: 1, handle: vec![9, 9] }] }.encode().unwrap(), |b| ProvenanceStream::decode(b).is_ok()),
    ];
    for (name, bytes, ok) in payloads {
        assert!(ok(&bytes), "{name} baseline");
        for n in 0..bytes.len() {
            assert!(!ok(&bytes[..n]), "{name} cut at {n} must not decode");
        }
        // a first word that claims billions of records, in a payload a few bytes long
        let mut lie = bytes.clone();
        lie[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!ok(&lie), "{name}: a count of u32::MAX");
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(!ok(&trailing), "{name}: trailing byte");
    }
}

#[test]
fn entity_ids_above_the_limit_and_invalid_utf8_are_refused() {
    let mut b = TextStream { records: vec![TextRecord { entity: id(0), x: 1.0, y: 2.0, height: 3.0, rotation: 0.0, text: "ab".into() }] }.encode().unwrap();
    b[4..8].copy_from_slice(&(1u32 << 28).to_le_bytes());
    assert_eq!(TextStream::decode(&b).unwrap_err(), ComponentError::BadEntityId(1 << 28));
    let mut b = LayerStream { names: vec!["ab".into()], assignments: vec![] }.encode().unwrap();
    let n = b.len();
    b[n - 2..].copy_from_slice(&[0xff, 0xfe]);
    assert_eq!(LayerStream::decode(&b).unwrap_err(), ComponentError::BadUtf8);
}

#[test]
fn random_corruption_of_payloads_never_panics() {
    let good = vec![
        LayerStream { names: vec!["A".into(), "B".into()], assignments: vec![(id(0), 1)] }.encode().unwrap(),
        HatchStream { records: vec![HatchRecord { boundary: vec![id(0)], fill: Fill::Pattern { name: "P".into(), angle: 1.0, scale: 1.0, rgba: 2 } }] }.encode().unwrap(),
        TextStream { records: vec![TextRecord { entity: id(0), x: 1.0, y: 2.0, height: 3.0, rotation: 0.0, text: "hi".into() }] }.encode().unwrap(),
    ];
    let mut x: u64 = 0xfeed_beef_1234_5678;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..3000 {
        for g in &good {
            let mut b = g.clone();
            for _ in 0..1 + (next() % 3) as usize {
                let at = (next() % b.len() as u64) as usize;
                b[at] = next() as u8;
            }
            let _ = LayerStream::decode(&b);
            let _ = HatchStream::decode(&b);
            let _ = TextStream::decode(&b);
            let _ = LineStyleStream::decode(&b);
            let _ = ProvenanceStream::decode(&b);
        }
    }
}
