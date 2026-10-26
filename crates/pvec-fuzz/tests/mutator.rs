//! Issue P7.8: the structure-aware mutator spends executions past the checksum.

mod support;

use pvec_fuzz::mutate::mutate;
use pvec_fuzz::seeds::{Source, corpus};
use support::reads;

fn sealed_seeds() -> Vec<Vec<u8>> {
    // the known-truth fixtures are written sealed: block checksums and the artifact digest
    corpus().into_iter().filter(|s| s.source == Source::KnownTruth).map(|s| s.bytes).collect()
}

fn gated(bytes: &[u8]) -> bool {
    matches!(reads(bytes), Err(rule) if rule.starts_with("INTEGRITY"))
}

#[test]
fn a_mutant_differs_from_its_seed_stays_inside_the_length_it_was_given_and_is_reproducible() {
    for seed in sealed_seeds().iter().take(10) {
        for n in 0..20u64 {
            let (mut a, mut b) = (seed.clone(), seed.clone());
            mutate(&mut a, seed.len() + 4096, n);
            mutate(&mut b, seed.len() + 4096, n);
            assert_eq!(a, b, "same seed, same mutant");
            assert!(a.len() <= seed.len() + 4096);
        }
        let changed = (0..20u64)
            .filter(|n| {
                let mut m = seed.clone();
                mutate(&mut m, seed.len() + 4096, *n);
                m != *seed
            })
            .count();
        assert!(changed >= 18, "only {changed} of 20 mutants differ");
    }
}

#[test]
fn most_mutants_pass_the_checksum_gate_where_most_plain_byte_flips_do_not() {
    let seeds = sealed_seeds();
    let (mut structured, mut plain, mut total) = (0, 0, 0);
    for seed in &seeds {
        for n in 0..40u64 {
            total += 1;
            let mut m = seed.clone();
            mutate(&mut m, seed.len() + 4096, n);
            structured += usize::from(gated(&m));
            let mut flipped = seed.clone();
            let at = (n as usize * 7919) % flipped.len();
            flipped[at] ^= 0x10;
            plain += usize::from(gated(&flipped));
        }
    }
    assert!(structured * 5 < total, "{structured} of {total} mutants stopped at the checksum");
    assert!(plain * 2 > total, "only {plain} of {total} plain flips stopped at the checksum");
}

#[test]
fn mutants_reach_more_than_one_kind_of_refusal_behind_the_gate() {
    let mut rules = std::collections::BTreeSet::new();
    for seed in sealed_seeds() {
        for n in 0..60u64 {
            let mut m = seed.clone();
            mutate(&mut m, seed.len() + 4096, n);
            rules.insert(reads(&m).err().unwrap_or_else(|| "ok".into()));
        }
    }
    let behind: Vec<&String> = rules.iter().filter(|r| !r.starts_with("INTEGRITY")).collect();
    assert!(behind.len() >= 8, "the mutants only reach {behind:?}");
}
