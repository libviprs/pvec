//! The compatibility matrix (issue P7.6): readers built at two capability sets crossed with files
//! written at both, and the outcome of every pair declared in `tests/compat/matrix.txt` rather than
//! computed here. The rules are `docs/COMPATIBILITY.md`; changing what a pair does turns its case red.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
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
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/compat/matrix.txt"))
        .expect("tests/compat/matrix.txt");
    let mut out: Vec<(String, Case)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(name) = line.strip_prefix("case ") {
            out.push((name.to_string(), Case::new()));
        } else {
            let (k, v) = line.split_once('=').unwrap_or_else(|| panic!("not a key = value line: {line}"));
            out.last_mut().expect("a line before any case").1.insert(k.trim().into(), v.trim().into());
        }
    }
    out
}

fn version(text: &str) -> Version {
    let mut p = text.split('.').map(|n| n.parse::<u16>().expect("a number"));
    Version::new(p.next().unwrap(), p.next().unwrap_or(0), p.next().unwrap_or(0))
}

fn list(text: Option<&String>) -> Vec<String> {
    text.map(|t| t.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default()
}

fn reader_named(name: &str) -> ReaderCapabilities {
    let mut r = ReaderCapabilities::v1();
    match name {
        "s_old" => {
            r = (14..=20).fold(r, |r, id| r.without(CapabilityId::new(id)));
            r.version = Version::new(1, 0, 0);
        }
        "s_new" => {
            r = r.with(CapabilityId::new(99), 3, 1);
            r.version = Version::new(1, 1, 0);
        }
        "v1" => {}
        other => panic!("unknown reader {other}"),
    }
    r
}

fn file_for(case: &Case) -> Art {
    let base = match case["base"].as_str() {
        "mixed" => art::from_input_with(&mixed(), SEALED),
        other => panic!("unknown base {other}"),
    };
    rebuild(&base, Integrity::Full, |m, blocks| {
        for spec in list(case.get("caps")) {
            let mut p = spec.split(':');
            let id = CapabilityId::new(p.next().unwrap().parse().unwrap());
            let requirement = match p.next().unwrap() {
                "required" => Requirement::Required,
                "optional" => Requirement::Optional,
                other => panic!("{other}"),
            };
            let d = Declared { id, requirement, version: version(p.next().unwrap()) };
            m.capabilities.retain(|c| c.id != id);
            m.capabilities.push(d);
        }
        m.capabilities.sort_by_key(|d| d.id);
        if let Some(v) = case.get("min_reader") {
            m.min_reader = version(v);
        }
        if let Some(g) = case.get("garbage") {
            let mut p = g.split(':').map(|n| n.parse::<u64>().unwrap());
            let (kind, capability, len) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
            let mut x = 0x9E37_79B9_7F4A_7C15u64;
            let stored = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x >> 24) as u8
                })
                .collect();
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
                stored,
            });
        }
    })
}

fn reason_name(r: SkipReason) -> &'static str {
    match r {
        SkipReason::NotImplemented => "not_implemented",
        SkipReason::NewerMajor => "newer_major",
        SkipReason::UnknownBlockKind => "unknown_block_kind",
        SkipReason::UnknownCodec => "unknown_codec",
    }
}

fn geometry(bytes: Vec<u8>, caps: ReaderCapabilities) -> Vec<(u16, u16, u32, Vec<u8>)> {
    let cfg = ReaderConfig { capabilities: caps, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(bytes), cfg).expect("the file opens");
    let mut out = Vec::new();
    for (i, e) in reader.entries().iter().enumerate() {
        if matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)) {
            let b = reader.read_block(BlockId(i as u32)).expect("a geometry block reads");
            out.push((e.kind, e.subtype, e.cell, b.as_bytes().to_vec()));
        }
    }
    out
}

#[test]
fn the_matrix_declares_at_least_four_reader_and_file_pairs_and_every_outcome_is_written_down() {
    let all = cases();
    assert!(all.len() >= 4);
    let pairs: std::collections::BTreeSet<_> =
        all.iter().map(|(_, c)| (c["reader"].clone(), c.get("caps").cloned().unwrap_or_default())).collect();
    assert!(pairs.len() >= 4, "{pairs:?}");
    for (name, c) in &all {
        assert!(matches!(c["outcome"].as_str(), "open" | "open_degraded" | "refuse"), "{name}");
        if c["outcome"] == "refuse" {
            assert!(c.contains_key("rule"), "{name} declares no rule");
        }
    }
}

#[test]
fn every_declared_pair_gives_its_declared_outcome() {
    let base = art::from_input_with(&mixed(), SEALED);
    for (name, case) in cases() {
        let file = file_for(&case);
        let reader = reader_named(&case["reader"]);
        let out = negotiate(&file.manifest, &file.entries, &reader);
        let gaps: Vec<String> = version_gaps(&file.manifest, &reader)
            .iter()
            .map(|g| format!("{}:{}", g.id, g.file))
            .collect();
        assert_eq!(gaps, list(case.get("gaps")), "{name}: version gaps");
        match (case["outcome"].as_str(), &out) {
            ("open", Outcome::Open) => {}
            ("open_degraded", Outcome::OpenDegraded(p)) => {
                let skipped: Vec<String> = p.skipped_ids().iter().map(u32::to_string).collect();
                assert_eq!(skipped, list(case.get("skipped")), "{name}: skipped ids");
                let reasons: Vec<&str> = p
                    .skipped_ids()
                    .iter()
                    .map(|id| {
                        let by_cap = p.capabilities.iter().find(|c| c.id.get() == *id).map(|c| c.reason);
                        let by_block =
                            p.blocks.iter().find(|b| b.capability.map(|c| c.get()) == Some(*id)).map(|b| b.reason);
                        reason_name(by_cap.or(by_block).unwrap())
                    })
                    .collect();
                assert_eq!(reasons, list(case.get("reasons")), "{name}: skip reasons");
            }
            ("refuse", Outcome::Refuse(r)) => {
                assert_eq!(r.rule(), case["rule"], "{name}: rule");
                if let Some(id) = case.get("named") {
                    assert!(r.to_string().contains(&format!("id {id}")), "{name}: {r}");
                }
            }
            (expected, got) => panic!("{name}: declared {expected}, got {got:?}"),
        }
        if case["outcome"] != "refuse" {
            if case.get("geometry").map(String::as_str) == Some("same") {
                assert_eq!(
                    geometry(file.bytes.clone(), reader.clone()),
                    geometry(base.bytes.clone(), ReaderCapabilities::v1()),
                    "{name}: geometry"
                );
            }
        } else {
            let cfg = ReaderConfig { capabilities: reader, ..ReaderConfig::default() };
            let e = ContainerReader::open(MemorySource::new(file.bytes.clone()), cfg).err().expect(&name);
            assert_eq!(e.rule(), case["rule"], "{name}: the open refuses with the same rule");
        }
    }
}

#[test]
fn a_block_of_pure_garbage_with_a_valid_length_is_skipped_and_never_parsed() {
    let (_, case) = cases().into_iter().find(|(n, _)| n == "old-reader-new-file-unknown-optional-block").unwrap();
    let file = file_for(&case);
    let cfg = ReaderConfig { capabilities: reader_named("s_old"), ..ReaderConfig::default() };
    let r = ContainerReader::open(MemorySource::new(file.bytes), cfg).expect("opens");
    let report = r.partial_open().expect("a report");
    assert_eq!(report.blocks.len(), 1);
    assert_eq!(report.blocks[0].reason, SkipReason::UnknownBlockKind);
    r.validate().expect("the file validates around the garbage");
}
