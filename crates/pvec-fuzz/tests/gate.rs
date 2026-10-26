//! Issue P7.8: the fuzz-only `no-checksum-gate` flag.

mod support;

use pvec_fuzz::{fuzz_directory_traversal, reseal};
use pvec_testkit::hostile::{bases, catalog};
use support::reads;

fn flipped_payload() -> Vec<u8> {
    let b = bases(&pvec_corpus::Bundle::vendored().unwrap());
    let c = catalog(&b);
    let case = c.iter().find(|c| c.name == "integrity/block_payload_bit_flip_in_a_sealed_file").unwrap();
    case.bytes(&b[case.base])
}

#[test]
fn a_flipped_payload_byte_is_stopped_at_the_gate_unless_the_input_is_resealed() {
    let bad = flipped_payload();
    let rule = reads(&bad).unwrap_err();
    assert!(rule.starts_with("INTEGRITY"), "{rule}");
    let resealed = reseal(&bad).expect("it still lays out as a container");
    let behind = reads(&resealed);
    assert!(!matches!(&behind, Err(r) if r.starts_with("INTEGRITY")), "{behind:?}");
    assert_eq!(reseal(&resealed).as_deref(), Some(resealed.as_slice()), "resealing is idempotent");
}

#[test]
fn input_that_is_not_a_container_is_not_resealed() {
    assert_eq!(reseal(&[]), None);
    assert_eq!(reseal(&[7; 500]), None);
}

#[test]
fn the_flag_decides_whether_the_targets_see_the_gate() {
    let word = fuzz_directory_traversal(&flipped_payload());
    if cfg!(feature = "no-checksum-gate") {
        assert!(!word.starts_with("INTEGRITY"), "past the gate: {word}");
    } else {
        assert!(word.starts_with("INTEGRITY"), "at the gate: {word}");
    }
}
