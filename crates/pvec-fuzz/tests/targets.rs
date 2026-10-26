//! Issue P7.8: the six targets exist, are wired to libFuzzer, and survive their seeds.
#![allow(unsafe_code)]

mod support;

use pvec_fuzz::seeds::{corpus, for_target};
use pvec_fuzz::{TARGETS, fuzz_header};
use support::*;

#[global_allocator]
static ALLOCATOR: counting_alloc::Counting = counting_alloc::Counting;

const NAMES: [&str; 6] = [
    "fuzz_header",
    "fuzz_directory_traversal",
    "fuzz_immediate_blocks",
    "fuzz_advanced_blocks",
    "fuzz_decompress_boundary",
    "fuzz_component_streams",
];

#[test]
fn the_six_targets_are_the_ones_the_issue_names() {
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
        assert!(inputs.len() >= 20, "{name} has only {} seeds", inputs.len());
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
