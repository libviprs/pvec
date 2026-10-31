//! Issue P7.8 and issue #218: the targets exist, are wired to libFuzzer, and survive their seeds.
#![allow(unsafe_code)]

mod support;

use pvec_fuzz::seeds::{corpus, for_target};
use pvec_fuzz::{TARGETS, fuzz_header};
use support::*;

#[global_allocator]
static ALLOCATOR: counting_alloc::Counting = counting_alloc::Counting;

const NAMES: [&str; 8] = [
    "fuzz_header",
    "fuzz_directory_traversal",
    "fuzz_immediate_blocks",
    "fuzz_advanced_blocks",
    "fuzz_decompress_boundary",
    "fuzz_component_streams",
    "fuzz_raster_index",
    "fuzz_raster_headers",
];

#[test]
fn the_targets_are_the_ones_the_issues_name() {
    let names: Vec<&str> = TARGETS.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, NAMES);
}

#[test]
fn each_target_has_a_libfuzzer_entry_point_in_the_fuzz_workspace() {
    let fuzz = repo().join("fuzz");
    let manifest = std::fs::read_to_string(fuzz.join("Cargo.toml")).expect("fuzz/Cargo.toml");
    assert!(manifest.contains("[workspace]"), "fuzz/ is a workspace of its own");
    assert!(manifest.contains("libfuzzer-sys"));
    for name in NAMES {
        let entry = std::fs::read_to_string(fuzz.join("fuzz_targets").join(format!("{name}.rs")))
            .unwrap_or_else(|_| panic!("fuzz/fuzz_targets/{name}.rs"));
        assert!(entry.contains("fuzz_target!") && entry.contains(&format!("pvec_fuzz::{name}")), "{name}");
        assert!(manifest.contains(&format!("name = \"{name}\"")), "fuzz/Cargo.toml has no bin {name}");
    }
    let root = std::fs::read_to_string(repo().join("Cargo.toml")).unwrap();
    assert!(root.contains("exclude = [\"spikes\", \"fuzz\"]"), "libfuzzer-sys stays out of the root lock");
    assert!(!std::fs::read_to_string(repo().join("Cargo.lock")).unwrap().contains("libfuzzer-sys"));
}

#[test]
fn every_target_survives_every_input_made_from_the_seeds() {
    let seeds = corpus();
    for (name, target) in TARGETS {
        let inputs = for_target(name, &seeds);
        assert!(inputs.len() >= 5, "{name} has only {} seeds", inputs.len());
        for (i, input) in inputs.iter().enumerate() {
            assert_eq!(panics(|| drop(target(input))), None, "{name} seed {i}");
        }
    }
}

#[test]
fn a_valid_input_reaches_the_parser_a_target_is_for_and_a_corrupt_one_is_refused_by_name() {
    let seeds = corpus();
    for (name, target) in TARGETS {
        let inputs = for_target(name, &seeds);
        let words: Vec<String> = inputs.iter().map(|i| target(i)).collect();
        assert!(
            words.iter().any(|w| w == "ok"),
            "{name}: no seed is accepted: {:?}",
            &words[..words.len().min(8)]
        );
        let refused: std::collections::BTreeSet<&String> = words.iter().filter(|w| *w != "ok").collect();
        assert!(!refused.is_empty(), "{name}: no seed is refused");
    }
}

#[test]
fn the_targets_take_empty_and_tiny_and_random_inputs_without_a_panic() {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut inputs: Vec<Vec<u8>> = vec![vec![], vec![0], vec![0xFF; 3], vec![0; 4096]];
    for len in [1usize, 7, 8, 63, 64, 127, 128, 129, 1000] {
        inputs.push(
            (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x >> 11) as u8
                })
                .collect(),
        );
    }
    for (name, target) in TARGETS {
        for input in &inputs {
            assert_eq!(panics(|| drop(target(input))), None, "{name} on {} bytes", input.len());
        }
    }
}

#[test]
fn the_header_target_stays_inside_a_small_allocation_whatever_the_manifest_claims() {
    let seeds = corpus();
    for input in for_target("fuzz_header", &seeds) {
        let bytes = metered(&mut || drop(fuzz_header(&input)));
        assert!(bytes <= 8 * input.len() + (32 << 10), "{bytes} bytes for {} input bytes", input.len());
    }
}

#[test]
fn the_hardening_report_names_every_target_and_says_what_was_not_run() {
    let doc = std::fs::read_to_string(repo().join("docs/FUZZING.md")).expect("docs/FUZZING.md");
    for name in NAMES {
        assert!(doc.contains(&format!("`{name}`")), "docs/FUZZING.md does not name {name}");
    }
    for needle in
        ["Seed corpus", "native arm64", "native x86_64", "## Findings", "has not run", "no-checksum-gate"]
    {
        assert!(doc.contains(needle), "docs/FUZZING.md does not say {needle}");
    }
}

#[test]
fn the_raster_index_target_reaches_its_parser_and_each_rule_it_is_for() {
    let seeds = corpus();
    let inputs = for_target("fuzz_raster_index", &seeds);
    assert!(inputs.len() >= 20, "only {} seeds", inputs.len());
    let words: std::collections::BTreeSet<String> =
        inputs.iter().map(|i| pvec_fuzz::fuzz_raster_index(i)).collect();
    for want in ["ok", "RASTER-02", "LIMIT-14"] {
        assert!(words.contains(want), "no seed reaches {want}: {words:?}");
    }
}

#[test]
fn an_index_that_parses_is_the_one_the_writer_would_write() {
    // the target asserts it, so a seed that parses and does not re-encode to itself would be a crash
    let seeds = corpus();
    for input in for_target("fuzz_raster_index", &seeds) {
        assert_eq!(panics(|| drop(pvec_fuzz::fuzz_raster_index(&input))), None);
    }
}

/// The `(kind, capability)` of every directory entry of `bytes`, when its header, manifest and directory parse.
fn entry_pairs(bytes: &[u8]) -> Vec<(u16, u16)> {
    use pvec_container::directory::parse_directory;
    use pvec_container::{DirectoryPointer, Header, Manifest};
    let len = bytes.len() as u64;
    let Ok(header) = Header::parse(&bytes[..bytes.len().min(4096)], len) else { return Vec::new() };
    let (m0, m1) =
        (header.manifest_offset as usize, (header.manifest_offset + header.manifest_length) as usize);
    let Some(m) = bytes.get(m0..m1) else { return Vec::new() };
    let Ok(manifest) = Manifest::parse(m, &header, len) else { return Vec::new() };
    let Some(p) = manifest.directories.iter().find(|d| d.kind == DirectoryPointer::KIND_BLOCK) else {
        return Vec::new();
    };
    let Some(t) = bytes.get(p.offset as usize..(p.offset + p.length) as usize) else { return Vec::new() };
    parse_directory(t, u64::from(header.header_len), len)
        .map(|es| es.iter().map(|e| (e.kind, e.capability)).collect())
        .unwrap_or_default()
}

#[test]
fn the_directory_traversal_seeds_carry_kind_9_and_10_entries_with_capability_21_0_and_an_unregistered_one() {
    // issue 216: this is where the traversal changed, so the corpus has the capability-owned kinds in it
    let seeds = corpus();
    let seen: std::collections::BTreeSet<(u16, u16)> =
        for_target("fuzz_directory_traversal", &seeds).iter().flat_map(|bytes| entry_pairs(bytes)).collect();
    for kind in [9u16, 10] {
        for capability in [21u16, 0, 77] {
            assert!(
                seen.contains(&(kind, capability)),
                "no seed has kind {kind} with capability {capability}"
            );
        }
    }
}

#[test]
fn fuzz_component_streams_is_seeded_with_one_valid_raster_placement_and_decodes_it() {
    // issue 217: the target decodes the raster stream, so the corpus has a valid placement in a set
    use pvec_capabilities::{Reader, Registry, Version};
    use pvec_components::{RASTER_CAPABILITY, decode_rasters};
    let seeds = corpus();
    let inputs = for_target("fuzz_component_streams", &seeds);
    let registry = Registry::v1();
    let reader = Reader { version: Version::new(1, 1, 0), implemented: vec![RASTER_CAPABILITY] };
    let placements = inputs
        .iter()
        .filter_map(|set| pvec_components::read_streams(set, &registry, &reader).ok())
        .flat_map(|opened| opened.streams)
        .filter(|s| s.capability == RASTER_CAPABILITY)
        .filter_map(|s| decode_rasters(&s.payload).ok())
        .map(|p| p.len())
        .sum::<usize>();
    assert!(placements >= 1, "no seed carries a valid raster placement");
}

#[test]
fn the_raster_headers_target_reaches_ok_and_each_rule_it_is_for() {
    let seeds = corpus();
    let inputs = for_target("fuzz_raster_headers", &seeds);
    // one tile of each codec and variant libviprs writes, at three edges, and the hostile corpus
    assert!(inputs.len() >= 36 + 13, "only {} seeds", inputs.len());
    let words: std::collections::BTreeSet<String> =
        inputs.iter().map(|i| pvec_fuzz::fuzz_raster_headers(i)).collect();
    for want in ["ok", "RASTER-03", "RASTER-04", "LIMIT-15"] {
        assert!(words.contains(want), "no seed reaches {want}: {words:?}");
    }
}

#[test]
fn the_raster_headers_target_takes_a_frame_shorter_than_its_header_and_a_header_alone() {
    for n in 0..4 {
        assert_eq!(panics(|| drop(pvec_fuzz::fuzz_raster_headers(&vec![1u8; n]))), None);
    }
    assert_eq!(pvec_fuzz::fuzz_raster_headers(&[]), "short");
}
