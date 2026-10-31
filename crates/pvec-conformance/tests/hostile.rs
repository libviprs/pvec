//! Issue P7.7 and the hostile corpus of P2.7: every hostile fixture is refused with exit 2 and a named
//! code, never a panic, and inside the P4.8 memory budget.
#![allow(unsafe_code)]

mod support;

#[path = "../../pvec-testkit/tests/support/counting_alloc.rs"]
mod counting_alloc;

use pvec_corpus::Bundle;
use pvec_testkit::hostile::{
    Base, Case, Expect, Limits, Meter, Reader, Refusal, Requires, Verdict, bases, catalog, run,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use support::*;

#[global_allocator]
static ALLOCATOR: counting_alloc::Counting = counting_alloc::Counting;

struct Counting;

impl Meter for Counting {
    fn measure(&self, f: &mut dyn FnMut()) -> usize {
        let before = counting_alloc::requested();
        f();
        counting_alloc::requested() - before
    }
}

/// The tool's `validate` as a reader the corpus can be run against.
struct Cli;

impl Reader for Cli {
    fn name(&self) -> String {
        "pvec validate".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse, Requires::Limits, Requires::Integrity, Requires::Negotiation]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        let out = pvec_conformance::commands::validate(bytes, "-");
        match out.error_code {
            None => {
                assert_eq!(out.exit, 0);
                Verdict::Accepted
            }
            Some(rule) => {
                assert!(matches!(out.exit, 2 | 3), "a refusal exits 2 or 3, not {}", out.exit);
                Verdict::Refused(Refusal { rule, detail: out.human })
            }
        }
    }
}

fn corpus() -> (Vec<Base>, Vec<Case>) {
    let b = bases(&Bundle::vendored().unwrap());
    let c = catalog(&b);
    (b, c)
}

#[test]
fn the_whole_hostile_corpus_is_satisfied_without_a_panic_a_hang_or_an_unbounded_allocation() {
    let (b, c) = corpus();
    let report = run(Arc::new(Cli), Arc::new(Counting), Arc::new(b), Arc::new(c), Limits::default());
    assert!(report.passed(), "{}", report.summary());
    assert!(report.ran > 10_000, "ran only {}", report.ran);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn each_family_of_hostile_fixture_exits_2_with_a_distinct_named_code() {
    let (b, c) = corpus();
    let picks = [
        ("overflowed offset", "offsets/block_offset_plus_length_wraps"),
        ("oversized count", "counts/hot_entities_u32_max"),
        ("decompression bomb", "decompression/ratio_beyond_budget"),
        ("nesting abuse", "nesting/chain_deeper_than_maximum"),
        ("malformed knot array", "knots/wrong_length_shorter"),
        ("invalid weights", "weights/count_short"),
        ("checksum failure", "integrity/block_payload_bit_flip_with_only_checksums"),
        ("truncated file", "truncation/in_block_payload"),
    ];
    let mut codes = BTreeSet::new();
    for (what, name) in picks {
        let case = c.iter().find(|x| x.name == name).unwrap_or_else(|| panic!("no case {name}"));
        let path = file("hostile.pvec", &case.bytes(&b[case.base]));
        let run = pvec(&["validate", "--format", "json", &path]);
        assert_eq!(run.code, 2, "{what}: {}{}", run.stdout, run.stderr);
        assert!(!run.stderr.contains("internal error"), "{what}: panicked");
        let doc = run.json();
        support::json::validate(&schema(), &doc).unwrap();
        let code = doc.at("error.code").str().to_string();
        let Expect::Refuse(allowed) = &case.expect else { panic!("{name} is not a refusal") };
        assert!(allowed.contains(&code.as_str()), "{what}: {code} is not one of {allowed:?}");
        codes.insert(code);
    }
    assert_eq!(codes.len(), picks.len(), "the eight attacks need eight distinct codes: {codes:?}");
}

/// Issue #219: the thirteen hostile tiles are corpus entries too. Each is a file whose geometry is fine and whose
/// raster holds the tile, and `validate` exits 0 with the raster skipped by the rule the corpus names.
#[test]
fn the_thirteen_hostile_tiles_are_skipped_by_validate_with_their_rule_and_the_file_conforms() {
    use pvec_testkit::raster_hostile::{file, hostile};
    let tiles = hostile();
    assert_eq!(tiles.len(), 13);
    for t in tiles {
        let out = pvec_conformance::commands::validate(&file(&t), "-");
        assert_eq!((out.exit, out.error_code.as_deref()), (0, None), "{}: {}", t.name, out.human);
        assert!(
            out.human.contains(&format!("raster 0 skipped: {}", t.rule.unwrap())),
            "{}: {}",
            t.name,
            out.human
        );
    }
}
