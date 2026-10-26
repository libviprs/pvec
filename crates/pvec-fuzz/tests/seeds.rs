//! Issue P7.8: the seed corpus is named, deterministic and says what it is made of.

use pvec_fuzz::seeds::{DEFERRED, Source, corpus};
use std::collections::BTreeSet;

#[test]
fn the_corpus_names_its_composition_and_every_source_is_present() {
    let seeds = corpus();
    let count = |s: Source| seeds.iter().filter(|x| x.source == s).count();
    assert!(count(Source::Hostile) >= 100, "hostile seeds: {}", count(Source::Hostile));
    assert!(count(Source::KnownTruth) >= 10, "known-truth seeds: {}", count(Source::KnownTruth));
    assert!(count(Source::Writer) >= 20, "writer seeds: {}", count(Source::Writer));
    let names: BTreeSet<&str> = seeds.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), seeds.len(), "seed names are unique");
    assert!(seeds.iter().all(|s| s.bytes.len() <= 1 << 20), "a seed over a mebibyte");
}

#[test]
fn the_corpus_is_deterministic() {
    let (a, b) = (corpus(), corpus());
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!((&x.name, x.source, &x.bytes), (&y.name, y.source, &y.bytes));
    }
}

#[test]
fn the_sources_that_are_not_on_main_are_listed_with_the_reason() {
    let text: String = DEFERRED.iter().map(|(a, b)| format!("{a} {b}")).collect();
    assert!(text.contains("P8.1") && text.contains("pvec-gold-fixtures"), "{text}");
}

#[test]
fn the_known_truth_seeds_are_the_fixtures_of_the_vendored_bundle() {
    let seeds = corpus();
    assert!(
        seeds.iter().any(|s| s.source == Source::KnownTruth && s.name.starts_with("known-truth/line-001"))
    );
}
