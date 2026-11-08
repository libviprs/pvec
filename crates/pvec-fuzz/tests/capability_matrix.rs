//! Issue P7.8: the capability matrix as an ordinary test, not a fuzz target. Readers built at the capability
//! sets `S_old` and `S_new` (and this build) are crossed with files written at both, and the pairs declared
//! in `tests/compat/matrix.txt` (P7.6) keep their declared outcome. Every other crossing has to be consistent
//! with the reader that opened it.

#[path = "../../pvec-container/tests/support/mod.rs"]
mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_container::blocks::AlignedBytes;
use pvec_container::directory::BlockKind;
use pvec_container::integrity::Integrity;
use pvec_container::negotiate::{Outcome, ReaderCapabilities, SkipReason, negotiate, version_gaps};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::RawBlock;
use std::collections::BTreeMap;
use support::art::{self, Art, SEALED, rebuild};
use support::fixtures::mixed;

type Case = BTreeMap<String, String>;

fn cases() -> Vec<(String, Case)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/compat/matrix.txt");
    let text = std::fs::read_to_string(path).expect("tests/compat/matrix.txt");
    let mut out: Vec<(String, Case)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(name) = line.strip_prefix("case ") {
            out.push((name.to_string(), Case::new()));
        } else {
            let (k, v) = line.split_once('=').expect("key = value");
            out.last_mut().unwrap().1.insert(k.trim().into(), v.trim().into());
        }
    }
    out
}

fn version(text: &str) -> Version {
    let mut p = text.split('.').map(|n| n.parse::<u16>().unwrap());
    Version::new(p.next().unwrap(), p.next().unwrap_or(0), p.next().unwrap_or(0))
}

fn list(text: Option<&String>) -> Vec<String> {
    text.map(|t| t.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

/// The four readers of the matrix: `s_raster` is a 1.1.0 reader with rasters and no calibration (ADR 0019).
fn readers() -> Vec<(&'static str, ReaderCapabilities)> {
    let mut old = (14..=22).fold(ReaderCapabilities::v1(), |r, id| r.without(CapabilityId::new(id)));
    old.version = Version::new(1, 0, 0);
    let mut new = ReaderCapabilities::v1().with(CapabilityId::new(99), 3, 1);
    new.version = Version::new(1, 1, 0);
    let mut raster = ReaderCapabilities::v1().without(CapabilityId::new(22));
    raster.version = Version::new(1, 1, 0);
    vec![("s_old", old), ("s_new", new), ("s_raster", raster), ("v1", ReaderCapabilities::v1())]
}

fn file_for(case: &Case) -> Art {
    let base = art::from_input_with(&mixed(), SEALED);
    rebuild(&base, Integrity::Full, |m, blocks| {
        for spec in list(case.get("caps")) {
            let mut p = spec.split(':');
            let id = CapabilityId::new(p.next().unwrap().parse().unwrap());
            let requirement =
                if p.next().unwrap() == "required" { Requirement::Required } else { Requirement::Optional };
            m.capabilities.retain(|c| c.id != id);
            m.capabilities.push(Declared { id, requirement, version: version(p.next().unwrap()) });
        }
        m.capabilities.sort_by_key(|d| d.id);
        if let Some(v) = case.get("min_reader") {
            m.min_reader = version(v);
        }
        if let Some(g) = case.get("garbage") {
            let mut p = g.split(':').map(|n| n.parse::<u64>().unwrap());
            let (kind, capability, len) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
            blocks.push(RawBlock {
                kind: kind as u16,
                subtype: 0,
                cell: 0,
                count: 1,
                bounds: None,
                flags: 0,
                codec: 0,
                capability: capability as u16,
                uncompressed_length: len as u32,
                stored: (0..len).map(|i| (i * 37 + 11) as u8).collect(),
            });
        }
    })
}

fn word(r: SkipReason) -> &'static str {
    match r {
        SkipReason::NotImplemented => "not_implemented",
        SkipReason::NewerMajor => "newer_major",
        SkipReason::UnknownBlockKind => "unknown_block_kind",
        SkipReason::UnknownCodec => "unknown_codec",
    }
}

fn geometry(bytes: &[u8], caps: &ReaderCapabilities) -> Option<Vec<Vec<u8>>> {
    let cfg = ReaderConfig { capabilities: caps.clone(), ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(bytes.to_vec()), cfg).ok()?;
    let mut out = Vec::new();
    for (i, e) in reader.entries().iter().enumerate() {
        if matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)) {
            out.push(reader.read_block(BlockId(i as u32)).ok()?.as_bytes().to_vec());
        }
    }
    let _ = AlignedBytes::from_bytes(&[]);
    Some(out)
}

#[test]
fn the_matrix_covers_at_least_four_declared_pairs_and_each_keeps_its_declared_outcome() {
    let all = cases();
    assert!(all.len() >= 4);
    let rs = readers();
    for (name, case) in &all {
        let (_, reader) = rs.iter().find(|(n, _)| *n == case["reader"]).expect("a known reader");
        let file = file_for(case);
        let out = negotiate(&file.manifest, &file.entries, reader);
        match (case["outcome"].as_str(), &out) {
            ("open", Outcome::Open) => {}
            ("open_degraded", Outcome::OpenDegraded(p)) => {
                let skipped: Vec<String> = p.skipped_ids().iter().map(u32::to_string).collect();
                assert_eq!(skipped, list(case.get("skipped")), "{name}");
                let reasons: Vec<&str> = p
                    .skipped_ids()
                    .iter()
                    .map(|id| {
                        let by_cap = p.capabilities.iter().find(|c| c.id.get() == *id).map(|c| c.reason);
                        word(by_cap.unwrap_or_else(|| {
                            p.blocks
                                .iter()
                                .find(|b| b.capability.map(|c| c.get()) == Some(*id))
                                .unwrap()
                                .reason
                        }))
                    })
                    .collect();
                assert_eq!(reasons, list(case.get("reasons")), "{name}");
            }
            ("refuse", Outcome::Refuse(r)) => assert_eq!(r.rule(), case["rule"], "{name}"),
            (declared, got) => panic!("{name}: declared {declared}, got {got:?}"),
        }
        let gaps: Vec<String> =
            version_gaps(&file.manifest, reader).iter().map(|g| format!("{}:{}", g.id, g.file)).collect();
        assert_eq!(gaps, list(case.get("gaps")), "{name}: gaps");
    }
}

#[test]
fn every_crossing_of_a_reader_and_a_file_is_consistent_with_the_reader_that_opened_it() {
    let mut files: Vec<(String, Art)> = cases().iter().map(|(n, c)| (n.clone(), file_for(c))).collect();
    files.push(("old-writer".into(), art::lines_with(4, 1, SEALED)));
    files.push(("mixed".into(), art::from_input_with(&mixed(), SEALED)));
    let mut crossings = 0;
    for (reader_name, reader) in readers() {
        for (file_name, file) in &files {
            crossings += 1;
            let here = format!("{reader_name} x {file_name}");
            let out = negotiate(&file.manifest, &file.entries, &reader);
            let cfg = ReaderConfig { capabilities: reader.clone(), ..ReaderConfig::default() };
            let opened = ContainerReader::open(MemorySource::new(file.bytes.clone()), cfg);
            match &out {
                Outcome::Refuse(r) => {
                    let e =
                        opened.err().unwrap_or_else(|| panic!("{here}: negotiate refuses and open does not"));
                    assert_eq!(e.rule(), r.rule(), "{here}");
                }
                Outcome::Open | Outcome::OpenDegraded(_) => {
                    let r =
                        opened.unwrap_or_else(|e| panic!("{here}: negotiate opens and open refuses: {e}"));
                    r.validate().unwrap_or_else(|e| panic!("{here}: {e}"));
                    if let Outcome::OpenDegraded(p) = &out {
                        let declared: Vec<u32> =
                            file.manifest.capabilities.iter().map(|d| d.id.get()).collect();
                        assert!(
                            p.skipped_ids().iter().all(|id| declared.contains(id)),
                            "{here}: skipped an undeclared id"
                        );
                    }
                }
            }
            assert_eq!(negotiate(&file.manifest, &file.entries, &reader), out, "{here}: deterministic");
        }
    }
    assert_eq!(crossings, 4 * files.len());
}

#[test]
fn the_geometry_read_is_the_same_whichever_reader_opens_the_file() {
    let rs = readers();
    for (name, case) in cases() {
        let file = file_for(&case);
        let seen: Vec<_> = rs.iter().filter_map(|(_, r)| geometry(&file.bytes, r)).collect();
        for pair in seen.windows(2) {
            assert_eq!(pair[0], pair[1], "{name}: two readers decoded different geometry");
        }
    }
}

#[test]
fn the_matrix_turns_red_if_a_declared_outcome_changes() {
    // the same harness, pointed at a case whose declared outcome has been edited, must disagree
    let (_, mut case) =
        cases().into_iter().find(|(n, _)| n == "old-reader-new-file-unknown-required-capability").unwrap();
    case.insert("rule".into(), "NEGOTIATE-02".into());
    let rs = readers();
    let (_, reader) = rs.iter().find(|(n, _)| *n == case["reader"]).unwrap();
    let file = file_for(&case);
    let Outcome::Refuse(r) = negotiate(&file.manifest, &file.entries, reader) else { panic!() };
    assert_ne!(r.rule(), case["rule"]);
}
