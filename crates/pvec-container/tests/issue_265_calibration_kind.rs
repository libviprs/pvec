//! Issue #265 (#263 slice 1, ADR 0019): kind 11 is reserved for capability 22. A reader without 22 skips the kind
//! 11 entry from the directory alone and reports `Effect::Uncalibrated`, a reader with it is handed the entry, a
//! kind 11 entry naming any other capability is refused, 22 can never be declared required, and the step to
//! format 1.2.0 moves no byte of a file that has no calibration.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_container::integrity::Integrity;
use pvec_container::negotiate::{
    CAPABILITY_OWNED_KINDS, Effect, NEGOTIATE_RULES, Outcome, ReaderCapabilities, SkipReason, negotiate,
};
use pvec_container::writer::RawBlock;
use std::collections::BTreeMap;
use support::art::{self, Art, SEALED, rebuild};
use support::fixtures::{fixtures, mixed};
use support::{raster, sha256_hex};

fn cap(id: u32) -> CapabilityId {
    CapabilityId::new(id)
}

/// `mixed` plus one opaque kind 11 block naming `capability`, with 22 declared at `requirement`.
fn calibrated(requirement: Requirement, capability: u16) -> Art {
    let base = art::from_input_with(&mixed(), SEALED);
    rebuild(&base, Integrity::Full, |m, blocks| {
        m.capabilities.push(Declared { id: cap(22), requirement, version: Version::new(1, 0, 0) });
        m.capabilities.sort_by_key(|d| d.id);
        blocks.push(RawBlock {
            kind: 11,
            subtype: 0,
            cell: 0,
            count: 1,
            bounds: None,
            flags: 0,
            codec: 0,
            capability,
            uncompressed_length: 64,
            stored: (0..64u8).collect(),
        });
    })
}

#[test]
fn kind_11_is_owned_by_capability_22() {
    assert!(CAPABILITY_OWNED_KINDS.contains(&(11, "calibration", cap(22))), "{CAPABILITY_OWNED_KINDS:?}");
    let rule = NEGOTIATE_RULES.iter().find(|(r, _)| *r == "NEGOTIATE-06").unwrap().1;
    assert!(rule.contains("kind 11 belongs to capability 22"), "{rule}");
    let rule = NEGOTIATE_RULES.iter().find(|(r, _)| *r == "NEGOTIATE-04").unwrap().1;
    assert!(rule.contains("whose class is never required"), "{rule}");
}

#[test]
fn a_reader_without_22_skips_the_block_and_says_the_file_is_uncalibrated() {
    let file = calibrated(Requirement::Optional, 22);
    let reader = ReaderCapabilities::v1().without(cap(22));
    let Outcome::OpenDegraded(p) = negotiate(&file.manifest, &file.entries, &reader) else {
        panic!("a reader without 22 opens a calibrated file degraded")
    };
    let c = p.capabilities.iter().find(|c| c.id == cap(22)).expect("22 is reported");
    assert_eq!((c.reason, c.effect, c.name), (SkipReason::NotImplemented, Effect::Uncalibrated, Some("calibration")));
    let b = p.blocks.iter().find(|b| b.kind == 11).expect("the kind 11 entry is reported");
    assert_eq!((b.reason, b.capability), (SkipReason::UnknownBlockKind, Some(cap(22))));
    assert!(
        p.describe().contains("this file carries a scale or georeference this reader cannot show"),
        "{}",
        p.describe()
    );
}

#[test]
fn a_reader_with_22_is_handed_the_block() {
    let file = calibrated(Requirement::Optional, 22);
    assert_eq!(negotiate(&file.manifest, &file.entries, &ReaderCapabilities::v1()), Outcome::Open);
}

#[test]
fn a_kind_11_block_naming_another_capability_is_refused() {
    let file = calibrated(Requirement::Optional, 21);
    let Outcome::Refuse(r) = negotiate(&file.manifest, &file.entries, &ReaderCapabilities::v1()) else {
        panic!("kind 11 naming 21 is refused")
    };
    assert_eq!(r.rule(), "NEGOTIATE-06");
}

#[test]
fn twenty_two_declared_required_is_refused_at_negotiate_04() {
    let file = calibrated(Requirement::Required, 22);
    let Outcome::Refuse(r) = negotiate(&file.manifest, &file.entries, &ReaderCapabilities::v1()) else {
        panic!("a required 22 is refused")
    };
    assert_eq!(r.rule(), "NEGOTIATE-04");
}

/// The digests every fixture had at format 1.1.0 (`tests/golden/digests.txt` before #265). The step to 1.2.0
/// adds a capability nothing here uses, so not one byte may move.
#[test]
fn files_without_a_calibration_are_byte_identical_to_what_1_1_0_wrote() {
    let at_1_1_0: BTreeMap<&str, &str> = [
        ("minimal", "a162e93a67357aa059c627c070218c3d057ba0c44c3423148a2885326aa25846"),
        ("mixed", "a16f2153c99e986a7827bc2d1962f2ae7e2eea2cb9c6f348e80a589644863b30"),
        ("model-curves", "8a6a952df29271116feeb19a7ce815f8efb6f62700808633b6425474c7922103"),
        ("plan", "ad1094c0314db0996d279cb56b4e6e5b63254e0fff2ad909bfd28315b7e4c7cc"),
        ("raster-two", "96182e779094332ff3c6a96653e7cda5de16a10fb8d8bf9a5ad988f2f80cabdb"),
        ("spline-heavy", "3b0b8f61b25d7855ece3a84615e0407a1955b3d01dd40a90e3474c048c8f253a"),
    ]
    .into_iter()
    .collect();
    let mut here: BTreeMap<String, String> =
        fixtures().iter().map(|f| (f.name.to_string(), sha256_hex(&f.write().expect("the fixture writes")))).collect();
    here.insert("raster-two".to_string(), sha256_hex(&raster::two_rasters_file()));
    assert_eq!(here.len(), at_1_1_0.len());
    for (name, digest) in &here {
        assert_eq!(at_1_1_0.get(name.as_str()), Some(&digest.as_str()), "{name} moved");
    }
    assert_eq!(pvec_capabilities::FORMAT_VERSION, Version::new(1, 2, 0));
}
