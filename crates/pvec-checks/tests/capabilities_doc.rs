//! The generated capability document and the coupling to the ISO profile (issue P3.4).

use pvec_capabilities::*;
use pvec_checks::*;
use std::fs;

fn profile_sections() -> Vec<ProfileSection> {
    let text = fs::read_to_string(repo_root().join("docs/ISO_PROFILE.md")).unwrap();
    parse_profile(&text)
        .into_iter()
        .map(|e| {
            let number = e.title.split('.').next().unwrap().parse().unwrap();
            let constructs = e
                .fields
                .iter()
                .find(|(k, _)| k == "ISO 10303-42:2025 construct")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            ProfileSection { number, constructs }
        })
        .collect()
}

#[test]
fn the_v1_registry_matches_the_profile_document() {
    let sections = profile_sections();
    assert_eq!(sections.len(), 13, "the profile has thirteen numbered capability sections");
    Registry::v1().validate_against_profile(&sections).unwrap();
}

#[test]
fn every_profile_section_has_a_registry_entry() {
    let r = Registry::v1();
    for s in profile_sections() {
        assert!(
            r.entries().iter().any(|c| c.class == Class::Geometry && c.profile_section == s.number),
            "profile section {} has no registry entry",
            s.number
        );
    }
}

#[test]
fn the_committed_capabilities_document_is_exactly_the_generated_one() {
    let path = repo_root().join("docs/CAPABILITIES.md");
    let generated = render_markdown(&Registry::v1());
    if std::env::var_os("PVEC_BLESS_CAPABILITIES").is_some() {
        fs::write(&path, &generated).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/CAPABILITIES.md differs from the registry; regenerate it with PVEC_BLESS_CAPABILITIES=1 \
         cargo test -p pvec-checks --test capabilities_doc"
    );
}

#[test]
fn a_single_changed_byte_in_the_document_is_detected() {
    let generated = render_markdown(&Registry::v1());
    let mut bytes = generated.clone().into_bytes();
    let at = bytes.iter().position(|b| *b == b'1').unwrap();
    bytes[at] = b'2';
    assert_ne!(String::from_utf8(bytes).unwrap(), generated);
    assert_ne!(format!("{generated} "), generated);
    assert_ne!(generated.replacen('|', "!", 1), generated);
}
