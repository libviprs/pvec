//! Issue P2.7: the hostile and corrupt container corpus, and the harness a reader has to satisfy.
//! Each test is named for the acceptance criterion it holds.
//!
//! This binary installs a counting allocator, which is what makes "allocation is bounded before it
//! happens" a measurement and not a timing: see `tests/support/counting_alloc.rs`.
#![allow(unsafe_code)]

mod common;
mod support;

use common::bundle;
use pvec_corpus::sha256::hex_digest;
use pvec_testkit::hostile::{
    Base, Case, Expect, Failure, Family, Limits, Meter, Reader, ReferenceReader, Refusal, Requires, Verdict,
    bases, catalog, run,
};
use pvec_testkit::reference::{Budget, Options, read, read_with};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

#[global_allocator]
static ALLOCATOR: support::counting_alloc::Counting = support::counting_alloc::Counting;

struct Counting;

impl Meter for Counting {
    fn measure(&self, f: &mut dyn FnMut()) -> usize {
        let before = support::counting_alloc::requested();
        f();
        support::counting_alloc::requested() - before
    }
}

fn corpus() -> (Arc<Vec<Base>>, Arc<Vec<Case>>) {
    let b = bases(&bundle());
    let c = catalog(&b);
    (Arc::new(b), Arc::new(c))
}

fn is_sweep(c: &Case) -> bool {
    c.name.starts_with("truncation/prefix/") || c.name.starts_with("truncation/remove_byte/")
}

/// The whole corpus, sweeps included.
fn run_full(reader: impl Reader + 'static, limits: Limits) -> pvec_testkit::hostile::Report {
    let (b, c) = corpus();
    run(Arc::new(reader), Arc::new(Counting), b, c, limits)
}

/// The named cases only: what the tests of wrong readers run, so that a reader that is wrong about
/// every case does not have to be wrong about tens of thousands of them.
fn run_with(reader: impl Reader + 'static, limits: Limits) -> pvec_testkit::hostile::Report {
    let (b, c) = corpus();
    let named: Vec<Case> = c.iter().filter(|c| !is_sweep(c)).cloned().collect();
    run(Arc::new(reader), Arc::new(Counting), b, Arc::new(named), limits)
}

fn run_subset(
    reader: impl Reader + 'static,
    names: &[&str],
    limits: Limits,
) -> pvec_testkit::hostile::Report {
    let (b, c) = corpus();
    let picked: Vec<Case> = c.iter().filter(|c| names.contains(&c.name.as_str())).cloned().collect();
    assert_eq!(picked.len(), names.len(), "every named case exists: {names:?}");
    run(Arc::new(reader), Arc::new(Counting), b, Arc::new(picked), limits)
}

/// The named cases the issue's table asks for, one row per mutation it lists.
const NAMED: &[(Family, &str)] = &[
    // offsets: offset+length wrap, offset past EOF, overlapping blocks
    (Family::Offsets, "offsets/block_offset_plus_length_wraps"),
    (Family::Offsets, "offsets/block_offset_past_eof"),
    (Family::Offsets, "offsets/block_length_past_eof"),
    (Family::Offsets, "offsets/overlapping_blocks"),
    (Family::Offsets, "offsets/block_overlaps_manifest"),
    (Family::Offsets, "offsets/block_overlaps_directory"),
    (Family::Offsets, "offsets/block_offset_not_aligned"),
    (Family::Offsets, "offsets/header_manifest_offset_wraps"),
    (Family::Offsets, "offsets/header_manifest_length_wraps"),
    (Family::Offsets, "offsets/header_directory_table_past_eof"),
    (Family::Offsets, "offsets/manifest_directory_pointer_past_eof"),
    (Family::Offsets, "offsets/manifest_directory_pointer_wraps"),
    (Family::Offsets, "offsets/directory_ends_before_eof"),
    // counts: entity, control-point and knot counts above the maxima and above the file size
    (Family::Counts, "counts/hot_entities_above_file_size"),
    (Family::Counts, "counts/hot_entities_above_maximum"),
    (Family::Counts, "counts/hot_entities_u32_max"),
    (Family::Counts, "counts/hot_vertices_above_file_size"),
    (Family::Counts, "counts/records_above_file_size"),
    (Family::Counts, "counts/records_above_maximum"),
    (Family::Counts, "counts/arena_elements_u32_max"),
    (Family::Counts, "counts/arena_entries_u32_max"),
    (Family::Counts, "counts/control_points_u32_max"),
    (Family::Counts, "counts/knots_u32_max"),
    (Family::Counts, "counts/segments_above_arena"),
    (Family::Counts, "counts/directory_entry_count_lies"),
    (Family::Counts, "counts/manifest_capabilities_u32_max"),
    (Family::Counts, "counts/manifest_directories_u32_max"),
    (Family::Counts, "counts/manifest_string_u32_max"),
    (Family::Counts, "counts/stream_count_above_maximum"),
    (Family::Counts, "counts/stream_count_above_file_size"),
    // decompression: expansion ratio beyond the budget
    (Family::Decompression, "decompression/ratio_beyond_budget"),
    (Family::Decompression, "decompression/ratio_beyond_budget_declared_u32_max"),
    (Family::Decompression, "decompression/ratio_beyond_budget_on_every_block"),
    // nesting: self-referential composite curve, deep component nesting
    (Family::Nesting, "nesting/composite_names_itself"),
    (Family::Nesting, "nesting/composites_name_each_other"),
    (Family::Nesting, "nesting/chain_deeper_than_maximum"),
    (Family::Nesting, "nesting/segment_names_missing_record"),
    (Family::Nesting, "nesting/segment_list_empty"),
    // knots: non-monotonic, wrong length, interior multiplicity above degree + 1
    (Family::Knots, "knots/non_monotonic"),
    (Family::Knots, "knots/wrong_length_shorter"),
    (Family::Knots, "knots/wrong_length_longer"),
    (Family::Knots, "knots/control_point_count_changed"),
    (Family::Knots, "knots/interior_multiplicity_above_degree_plus_one"),
    // weights: zero, negative, non-finite
    (Family::Weights, "weights/zero"),
    (Family::Weights, "weights/negative"),
    (Family::Weights, "weights/nan"),
    (Family::Weights, "weights/infinite"),
    (Family::Weights, "weights/below_conditioning_floor"),
    (Family::Weights, "weights/missing_on_rational"),
    (Family::Weights, "weights/present_on_non_rational"),
    (Family::Weights, "weights/count_short"),
    (Family::Weights, "weights/count_long"),
    // domains: inverted, zero-length, non-finite
    (Family::Domains, "domains/knots_inverted"),
    (Family::Domains, "domains/knots_zero_length"),
    (Family::Domains, "domains/knot_nan"),
    (Family::Domains, "domains/knot_infinite"),
    (Family::Domains, "domains/control_point_nan"),
    (Family::Domains, "domains/control_point_infinite"),
    (Family::Domains, "domains/control_point_beyond_coordinate_range"),
    (Family::Domains, "domains/trim_nan"),
    (Family::Domains, "domains/trim_count_not_twice_segments"),
    (Family::Domains, "domains/hot_coordinate_nan"),
    (Family::Domains, "domains/hot_arc_sweep_infinite"),
    (Family::Domains, "domains/hot_polyline_vertex_nan"),
    (Family::Domains, "domains/hot_arc_radius_zero"),
    (Family::Domains, "domains/hot_arc_radius_negative"),
    (Family::Domains, "domains/hot_coordinate_beyond_coordinate_range"),
    (Family::Domains, "domains/hot_coordinate_subnormal"),
    (Family::Domains, "domains/hot_line_zero_length"),
    (Family::Domains, "domains/hot_polyline_repeated_vertex"),
    // integrity: artifact digest and block checksum mismatch
    (Family::Integrity, "integrity/artifact_digest_mismatch"),
    (Family::Integrity, "integrity/artifact_digest_present_but_zero"),
    (Family::Integrity, "integrity/block_checksum_table_declared_over_block_bytes"),
    // truncation: header, directory, block header, block payload
    (Family::Truncation, "truncation/at_zero"),
    (Family::Truncation, "truncation/in_magic"),
    (Family::Truncation, "truncation/in_header"),
    (Family::Truncation, "truncation/header_only"),
    (Family::Truncation, "truncation/in_manifest"),
    (Family::Truncation, "truncation/before_directory"),
    (Family::Truncation, "truncation/in_directory"),
    (Family::Truncation, "truncation/in_block_header"),
    (Family::Truncation, "truncation/in_block_payload"),
    (Family::Truncation, "truncation/last_byte"),
    // trailing bytes
    (Family::TrailingBytes, "trailing/one_zero_byte"),
    (Family::TrailingBytes, "trailing/eight_zero_bytes"),
    (Family::TrailingBytes, "trailing/junk_4kib"),
    (Family::TrailingBytes, "trailing/second_copy"),
    // unknown tags
    (Family::UnknownTags, "tags/unknown_required_block_kind"),
    (Family::UnknownTags, "tags/unknown_optional_block_kind_is_skipped"),
    (Family::UnknownTags, "tags/unknown_hot_kind"),
    (Family::UnknownTags, "tags/unknown_record_kind"),
    (Family::UnknownTags, "tags/unknown_payload_kind"),
    (Family::UnknownTags, "tags/unknown_codec_on_required_block"),
    (Family::UnknownTags, "tags/unknown_checksum_algorithm"),
    (Family::UnknownTags, "tags/unknown_length_unit"),
    (Family::UnknownTags, "tags/unknown_manifest_flag"),
    (Family::UnknownTags, "tags/unknown_directory_flag"),
    (Family::UnknownTags, "tags/unknown_record_flag"),
    (Family::UnknownTags, "tags/periodic_record_flag"),
    (Family::UnknownTags, "tags/unknown_capability_requirement"),
    (Family::UnknownTags, "tags/future_container_major_version"),
    (Family::UnknownTags, "tags/bad_magic"),
    (Family::UnknownTags, "tags/big_endian_marker"),
    (Family::UnknownTags, "tags/future_hot_version"),
    (Family::UnknownTags, "tags/future_arena_version"),
    (Family::UnknownTags, "tags/unknown_stream_set_version"),
    // corruption inside blocks, and of order and reserved fields
    (Family::Corruption, "corruption/hot_block_length_lies"),
    (Family::Corruption, "corruption/arena_block_length_lies"),
    (Family::Corruption, "corruption/records_block_length_lies"),
    (Family::Corruption, "corruption/hot_reserved_nonzero"),
    (Family::Corruption, "corruption/directory_reserved_nonzero"),
    (Family::Corruption, "corruption/header_reserved_nonzero"),
    (Family::Corruption, "corruption/hot_ids_not_ascending"),
    (Family::Corruption, "corruption/record_ids_not_ascending"),
    (Family::Corruption, "corruption/entity_in_two_blocks"),
    (Family::Corruption, "corruption/directory_entries_out_of_order"),
    (Family::Corruption, "corruption/polyline_run_offsets_decrease"),
    (Family::Corruption, "corruption/stream_length_lies"),
    (Family::Corruption, "corruption/stream_set_trailing_bytes"),
    (Family::Corruption, "corruption/stream_reserved_nonzero"),
    (Family::Corruption, "corruption/manifest_not_after_header"),
    (Family::Corruption, "corruption/directory_pointer_of_another_kind"),
    (Family::Corruption, "corruption/header_length_below_fixed_header"),
    (Family::Corruption, "corruption/capabilities_out_of_order"),
    (Family::Corruption, "corruption/record_count_disagrees_with_directory"),
    (Family::Corruption, "corruption/arena_block_missing"),
    (Family::Corruption, "corruption/directory_empty"),
    (Family::Corruption, "corruption/padding_nonzero_between_blocks"),
    (Family::Corruption, "corruption/padding_nonzero_before_directory"),
];

#[test]
fn every_row_of_the_issue_table_has_its_named_case_and_every_family_is_covered() {
    let (_, cases) = corpus();
    let by_name: BTreeMap<&str, &Case> = cases.iter().map(|c| (c.name.as_str(), c)).collect();
    for (family, name) in NAMED {
        let c = by_name.get(name).unwrap_or_else(|| panic!("no case named {name}"));
        assert_eq!(c.family, *family, "{name}");
    }
    for f in [
        Family::Offsets,
        Family::Counts,
        Family::Decompression,
        Family::Nesting,
        Family::Knots,
        Family::Weights,
        Family::Domains,
        Family::Integrity,
        Family::Truncation,
        Family::TrailingBytes,
        Family::UnknownTags,
        Family::Corruption,
    ] {
        assert!(cases.iter().any(|c| c.family == f), "no case in {f:?}");
    }
    // the sweeps: every proper prefix and every single removed byte of every base is some case
    let (bases, _) = corpus();
    let produced: BTreeSet<String> = cases.iter().map(|c| hex_digest(&c.bytes(&bases[c.base]))).collect();
    for b in bases.iter() {
        for len in 0..b.bytes.len() {
            assert!(
                produced.contains(&hex_digest(&b.bytes[..len])),
                "{}: no case cuts it to {len} bytes",
                b.name
            );
        }
        for at in 0..b.bytes.len() {
            let mut v = b.bytes.clone();
            v.remove(at);
            assert!(produced.contains(&hex_digest(&v)), "{}: no case removes byte {at}", b.name);
        }
    }
}

#[test]
fn cases_are_deterministic_unique_and_hermetic() {
    let digest = |cases: &[Case], bases: &[Base]| -> Vec<(String, String)> {
        cases.iter().map(|c| (c.name.clone(), hex_digest(&c.bytes(&bases[c.base])))).collect()
    };
    let (b1, c1) = corpus();
    let (b2, c2) = corpus();
    assert!(c1.len() > 100, "the corpus is not empty ({})", c1.len());
    assert_eq!(digest(&c1, &b1), digest(&c2, &b2), "building the corpus twice gives the same cases");
    let names: BTreeSet<&str> = c1.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), c1.len(), "case names are unique");
    // no two cases are the same file: each tests its own mutation
    let files: BTreeSet<String> = digest(&c1, &b1).into_iter().map(|(_, d)| d).collect();
    assert_eq!(files.len(), c1.len(), "two cases produce identical bytes");
    // hermetic: nothing here opens a socket, starts a process or reads the environment
    for (file, text) in [
        ("hostile/mod.rs", include_str!("../src/hostile/mod.rs")),
        ("hostile/bases.rs", include_str!("../src/hostile/bases.rs")),
        ("hostile/catalog.rs", include_str!("../src/hostile/catalog.rs")),
        ("hostile/harness.rs", include_str!("../src/hostile/harness.rs")),
        ("hostile/mutate.rs", include_str!("../src/hostile/mutate.rs")),
        ("reference.rs", include_str!("../src/reference.rs")),
    ] {
        for banned in ["std::net", "std::process", "Command::new", "std::env", "std::fs", "TcpStream"] {
            assert!(!text.contains(banned), "{file} mentions {banned}");
        }
    }
}

#[test]
fn each_case_is_one_effective_mutation_and_the_base_it_repairs_to_reads() {
    let (bases, cases) = corpus();
    for b in bases.iter() {
        read(&b.bytes).unwrap_or_else(|e| panic!("base {} is not a valid artifact: {e}", b.name));
    }
    assert!(!cases.is_empty());
    for c in cases.iter() {
        assert!(!c.ops.is_empty(), "{}: no mutation", c.name);
        let base = &bases[c.base].bytes;
        let mut at = base.clone();
        for (i, op) in c.ops.iter().enumerate() {
            let next = pvec_testkit::hostile::apply(&at, std::slice::from_ref(op));
            assert!(next != at, "{}: op {i} changes nothing", c.name);
            at = next;
        }
        assert!(at == c.bytes(&bases[c.base]));
        assert!(&at != base, "{}: the mutation is a no-op", c.name);
    }
}

#[test]
fn the_reference_reader_satisfies_every_case_it_provides() {
    let (_, cases) = corpus();
    let report = run_full(ReferenceReader::new(), Limits::default());
    assert!(report.passed(), "{}", report.summary());
    assert!(report.ran > 10_000, "ran only {}", report.ran);
    // the calibration of Limits::default: the reference's largest read, over the whole corpus
    let floor = Limits::default().alloc_floor;
    assert!(
        report.peak_allocation * 2 < floor,
        "the reference peaked at {}, the floor is {floor}",
        report.peak_allocation
    );
    let skipped: usize = report.skipped.values().sum();
    assert_eq!(report.ran + skipped, cases.len());
    // what a later issue has to provide is counted, not hidden
    assert_eq!(
        report.skipped.keys().copied().collect::<Vec<_>>(),
        vec![Requires::Integrity, Requires::Negotiation, Requires::Semantics],
        "the reference has parsers for everything else"
    );
    assert!(report.skipped[&Requires::Integrity] >= 3);
    assert!(report.skipped[&Requires::Negotiation] >= 1);
    assert!(report.skipped[&Requires::Semantics] >= 5);
}

#[test]
fn a_reader_without_a_budget_is_not_held_to_the_decompression_cases() {
    let reader = ReferenceReader { options: Options { check_nesting: true, budget: None } };
    let report = run_with(reader, Limits::default());
    assert!(report.passed(), "{}", report.summary());
    assert!(report.skipped[&Requires::Limits] >= 3, "{:?}", report.skipped);
}

#[test]
fn a_decompression_bomb_stops_at_the_configured_ratio_and_raising_it_is_the_only_way_through() {
    let names = ["decompression/ratio_beyond_budget", "decompression/ratio_beyond_budget_declared_u32_max"];
    let at_budget = run_subset(ReferenceReader::new(), &names, Limits::default());
    assert!(at_budget.passed(), "{}", at_budget.summary());
    assert_eq!(at_budget.ran, 2);
    // the same files under a budget that allows the declared expansion: the limit no longer
    // refuses them, so the case turns red, and what refuses them now is the container's own rule.
    let roomy = Budget { max_expansion_ratio: u64::MAX, max_decoded_bytes: u64::MAX };
    let raised = ReferenceReader { options: Options { check_nesting: true, budget: Some(roomy) } };
    let report = run_subset(raised, &names, Limits::default());
    assert_eq!(report.failures.len(), 2, "{}", report.summary());
    for (_, f) in &report.failures {
        assert!(matches!(f, Failure::WrongRule { got, .. } if got == "DIR-09"), "{f:?}");
    }
}

// ---------------------------------------------------------------- readers that are wrong

/// Accepts whatever it is given.
struct AcceptsAnything;
impl Reader for AcceptsAnything {
    fn name(&self) -> String {
        "accepts anything".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![
            Requires::Parse,
            Requires::Limits,
            Requires::Integrity,
            Requires::Negotiation,
            Requires::Semantics,
        ]
    }
    fn read(&self, _: &[u8]) -> Verdict {
        Verdict::Accepted
    }
}

/// Panics on every file, as `unwrap` on an attacker's length would.
struct Panics;
impl Reader for Panics {
    fn name(&self) -> String {
        "panics".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        let v: Vec<u8> = Vec::new();
        let _ = v[bytes.len()];
        Verdict::Accepted
    }
}

/// Never returns.
struct Hangs;
impl Reader for Hangs {
    fn name(&self) -> String {
        "hangs".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, _: &[u8]) -> Verdict {
        loop {
            std::thread::park_timeout(Duration::from_millis(50));
        }
    }
}

/// Reads like the reference, but says a rule no case allows.
struct WrongRule;
impl Reader for WrongRule {
    fn name(&self) -> String {
        "wrong rule".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        match ReferenceReader::new().read(bytes) {
            Verdict::Refused(r) => Verdict::Refused(Refusal { rule: "XX-00".into(), ..r }),
            v => v,
        }
    }
}

/// Refuses everything, the valid base included.
struct RefusesAll;
impl Reader for RefusesAll {
    fn name(&self) -> String {
        "refuses all".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, _: &[u8]) -> Verdict {
        Verdict::Refused(Refusal { rule: "DIR-06".into(), detail: "no".into() })
    }
}

/// Trusts a count it reads from the file and reserves room for it before checking it against the
/// bytes that are there.
struct TrustsTheCount;
impl Reader for TrustsTheCount {
    fn name(&self) -> String {
        "trusts the count".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        // the count of the first block, found the way a reader would: header, pointer table,
        // directory, first entry, block header
        let le = |at: usize, n: usize| -> Option<u64> {
            bytes.get(at..at + n).map(|b| b.iter().rev().fold(0u64, |a, x| (a << 8) | u64::from(*x)))
        };
        let declared = (|| {
            let table = le(40, 8)? as usize;
            let directory = le(table + 8, 8)? as usize;
            let block = le(directory, 8)? as usize;
            le(block + 8, 4)
        })()
        .unwrap_or(1) as usize;
        let reserved: Vec<u64> = Vec::with_capacity(declared.clamp(1 << 22, 1 << 23));
        std::hint::black_box(&reserved);
        ReferenceReader::new().read(bytes)
    }
}

fn parse_cases() -> Vec<Case> {
    corpus().1.iter().filter(|c| c.requires == Requires::Parse && !is_sweep(c)).cloned().collect()
}

#[test]
fn a_reader_that_accepts_hostile_files_fails_every_case_that_wants_a_refusal() {
    let report = run_with(AcceptsAnything, Limits::default());
    let wants_refusal =
        corpus().1.iter().filter(|c| !is_sweep(c) && matches!(c.expect, Expect::Refuse(_))).count();
    assert!(wants_refusal > 80, "{wants_refusal}");
    assert_eq!(report.failures.len(), wants_refusal, "{}", report.summary());
    assert!(report.failures.iter().all(|(_, f)| *f == Failure::Accepted));
}

#[test]
fn a_panic_is_a_failure_of_the_reader_and_the_run_carries_on() {
    let parse = parse_cases().len();
    assert!(parse > 80, "the corpus has {parse} named cases that need only the parsers");
    let report = run_with(Panics, Limits::default());
    assert_eq!(report.ran, parse);
    assert_eq!(report.failures.len(), parse, "{}", report.summary());
    assert!(
        report
            .failures
            .iter()
            .all(|(_, f)| matches!(f, Failure::Panicked(m) if m.contains("index out of bounds")))
    );
}

#[test]
fn a_hang_is_a_failure_and_the_run_carries_on() {
    let limits = Limits { case_timeout: Duration::from_millis(250), ..Limits::default() };
    let names = ["offsets/block_offset_past_eof", "knots/non_monotonic", "truncation/in_header"];
    let report = run_subset(Hangs, &names, limits);
    assert_eq!(report.ran, 3);
    assert_eq!(report.failures.len(), 3, "{}", report.summary());
    assert!(report.failures.iter().all(|(_, f)| *f == Failure::Hung));
}

#[test]
fn a_refusal_that_names_no_allowed_rule_is_a_failure() {
    let names = ["offsets/block_offset_past_eof", "knots/non_monotonic"];
    let report = run_subset(WrongRule, &names, Limits::default());
    assert_eq!(report.failures.len(), 2, "{}", report.summary());
    assert!(
        report.failures.iter().all(|(_, f)| matches!(f, Failure::WrongRule { got, .. } if got == "XX-00"))
    );
}

#[test]
fn a_reader_that_cannot_read_the_valid_file_does_not_pass_on_its_refusals() {
    // The case's own mutation is refused with a rule the case allows, so on that count it passes.
    // What fails it is the repair: the base the mutation was made from is refused too, which shows
    // the refusal was not about the mutation.
    let report = run_subset(RefusesAll, &["offsets/block_offset_past_eof"], Limits::default());
    assert_eq!(report.failures.len(), 1, "{}", report.summary());
    assert!(matches!(report.failures[0].1, Failure::RepairRefused(_)), "{:?}", report.failures);
}

#[test]
fn removing_the_nesting_guard_turns_exactly_the_nesting_cases_red() {
    let reader = ReferenceReader { options: Options { check_nesting: false, budget: Some(Budget::DEFAULT) } };
    let report = run_with(reader, Limits::default());
    let mut failed = report.failed_names();
    failed.sort_unstable();
    assert_eq!(
        failed,
        [
            "nesting/chain_deeper_than_maximum",
            "nesting/composite_names_itself",
            "nesting/composites_name_each_other"
        ],
        "{}",
        report.summary()
    );
    assert!(report.failures.iter().all(|(_, f)| *f == Failure::Accepted));
}

#[test]
fn a_file_declaring_billions_of_elements_is_refused_without_allocating_in_proportion() {
    let names = [
        "counts/hot_entities_u32_max",
        "counts/hot_entities_above_maximum",
        "counts/records_above_maximum",
        "counts/arena_elements_u32_max",
        "counts/control_points_u32_max",
        "counts/knots_u32_max",
        "counts/manifest_capabilities_u32_max",
    ];
    let report = run_subset(ReferenceReader::new(), &names, Limits::default());
    assert!(report.passed(), "{}", report.summary());
    assert_eq!(report.ran, names.len());
    // measured by the counting allocator: far below one byte per declared element (2^32 of them)
    assert!(report.peak_allocation < 1 << 20, "peak {}", report.peak_allocation);
}

#[test]
fn the_counting_allocator_sees_what_a_reader_allocates_and_catches_one_that_trusts_a_count() {
    let m = Counting;
    let seen = m.measure(&mut || {
        let v = vec![1u8; 4096];
        std::hint::black_box(&v);
    });
    assert!(seen >= 4096, "saw {seen}");
    let report = run_subset(TrustsTheCount, &["counts/hot_entities_u32_max"], Limits::default());
    assert_eq!(report.failures.len(), 1, "{}", report.summary());
    assert!(
        matches!(report.failures[0].1, Failure::Allocated { bytes, .. } if bytes >= 1 << 25),
        "{:?}",
        report.failures
    );
}

#[test]
fn the_decoded_size_ceiling_refuses_from_the_directory_alone() {
    let (bases, _) = corpus();
    let tiny = Options {
        check_nesting: true,
        budget: Some(Budget { max_expansion_ratio: 64, max_decoded_bytes: 16 }),
    };
    let e = read_with(&bases[0].bytes, &tiny).expect_err("a ceiling of 16 bytes refuses any artifact");
    assert_eq!(e.rule, "LIMIT-02");
    assert!(read_with(&bases[0].bytes, &Options::default()).is_ok());
}

/// How many proper prefixes of the bases `accepts` takes for a complete artifact.
fn accepted_prefixes(accepts: &dyn Fn(&[u8]) -> bool) -> usize {
    let (bases, _) = corpus();
    let mut n = 0;
    for b in bases.iter() {
        assert!(accepts(&b.bytes), "{}: the whole artifact is accepted", b.name);
        n += (0..b.bytes.len()).filter(|len| accepts(&b.bytes[..*len])).count();
    }
    n
}

#[test]
fn truncating_any_prefix_never_yields_a_successful_parse() {
    assert_eq!(accepted_prefixes(&|b| read(b).is_ok()), 0);
    let (bases, _) = corpus();
    assert!(bases.iter().map(|b| b.bytes.len()).sum::<usize>() > 5_000);
}

#[test]
fn the_prefix_sweep_catches_a_reader_that_stops_after_the_magic() {
    let lenient = |b: &[u8]| b.len() >= 64 && b.starts_with(&pvec_container::spec::MAGIC);
    assert!(accepted_prefixes(&lenient) > 1_000);
}

#[test]
fn removing_any_single_byte_never_yields_a_successful_parse() {
    let (bases, _) = corpus();
    for b in bases.iter() {
        for at in 0..b.bytes.len() {
            let mut v = b.bytes.clone();
            v.remove(at);
            assert!(read(&v).is_err(), "{}: removing byte {at} still parses", b.name);
        }
    }
}

#[test]
fn every_rule_a_case_pins_is_one_the_container_documents() {
    let (_, cases) = corpus();
    let docs =
        [include_str!("../../../docs/CONTAINER.md"), include_str!("../../../docs/COMPONENT_STREAMS.md")]
            .concat();
    let container_errors = include_str!("../../pvec-container/src/error.rs");
    let stream_errors = include_str!("../../pvec-components/src/error.rs");
    let reference = include_str!("../src/reference.rs");
    let policy = include_str!("../../../docs/NUMERIC_POLICY.md");
    let mut seen = BTreeSet::new();
    for c in cases.iter() {
        let Expect::Refuse(rules) = &c.expect else { continue };
        assert!(!rules.is_empty(), "{}: a refusal with no rule", c.name);
        for r in rules {
            if !seen.insert(*r) {
                continue;
            }
            let documented = if ["DIR-", "HOT-", "ARN-"].iter().any(|p| r.starts_with(p)) {
                docs.contains(&format!("| {r} |"))
            } else if ["STREAM-", "COMP-"].iter().any(|p| r.starts_with(p)) {
                stream_errors.contains(&format!("\"{r}\""))
            } else if ["LAYOUT-", "CONSIST-", "LIMIT-", "NEGOTIATE-"].iter().any(|p| r.starts_with(p)) {
                reference.contains(&format!("\"{r}\""))
            } else if r.starts_with("NP-") {
                policy.contains(&format!("### {r} "))
            } else if r.starts_with("INTEGRITY-") {
                true
            } else {
                container_errors.contains(&format!("    {r}"))
            };
            assert!(documented, "{}: rule {r} is not defined anywhere", c.name);
        }
    }
    assert!(seen.len() > 30, "the corpus exercises {} distinct rules", seen.len());
}

#[test]
fn case_names_start_with_their_family() {
    let (_, cases) = corpus();
    assert!(cases.len() > 100);
    for c in cases.iter() {
        let prefix = match c.family {
            Family::Offsets => "offsets/",
            Family::Counts => "counts/",
            Family::Decompression => "decompression/",
            Family::Nesting => "nesting/",
            Family::Knots => "knots/",
            Family::Weights => "weights/",
            Family::Domains => "domains/",
            Family::Integrity => "integrity/",
            Family::Truncation => "truncation/",
            Family::TrailingBytes => "trailing/",
            Family::UnknownTags => "tags/",
            Family::Corruption => "corruption/",
        };
        assert!(c.name.starts_with(prefix), "{} is in {:?}", c.name, c.family);
    }
    let _ = read_with; // the budgeted entry point is exercised through ReferenceReader
}

/// Panics on a file it accepts, as a reader that unwraps something on the success path would.
struct PanicsOnValid;
impl Reader for PanicsOnValid {
    fn name(&self) -> String {
        "panics on valid".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        match ReferenceReader::new().read(bytes) {
            Verdict::Accepted => panic!("an unwrap on the success path"),
            v => v,
        }
    }
}

/// Refuses everything and claims to skip unknown optional blocks.
struct RefusesEvenWhatItShouldSkip;
impl Reader for RefusesEvenWhatItShouldSkip {
    fn name(&self) -> String {
        "refuses what it should skip".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse, Requires::Negotiation]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        ReferenceReader::new().read(bytes)
    }
}

#[test]
fn a_reader_that_breaks_on_the_valid_file_is_caught_on_the_repair() {
    let report = run_subset(PanicsOnValid, &["offsets/block_offset_past_eof"], Limits::default());
    assert_eq!(report.failures.len(), 1, "{}", report.summary());
    assert!(matches!(&report.failures[0].1, Failure::RepairBroke(f) if matches!(**f, Failure::Panicked(_))));
}

#[test]
fn a_reader_that_claims_negotiation_must_accept_the_optional_block_it_does_not_know() {
    // the reference does not skip unknown optional blocks: it refuses them, naming NEGOTIATE-01
    let report = run_subset(
        RefusesEvenWhatItShouldSkip,
        &["tags/unknown_optional_block_kind_is_skipped"],
        Limits::default(),
    );
    assert_eq!(report.failures.len(), 1, "{}", report.summary());
    assert!(matches!(&report.failures[0].1, Failure::Rejected(r) if r.rule == "NEGOTIATE-01"));
}

#[test]
fn the_hot_parsers_let_semantically_hostile_values_through_so_a_reader_has_to_check_them() {
    // This is the gap the Semantics cases exist for: each of these files parses today.
    let (bases, cases) = corpus();
    let gaps: Vec<&Case> = cases.iter().filter(|c| c.requires == Requires::Semantics).collect();
    assert!(gaps.len() >= 6, "{}", gaps.len());
    for c in gaps {
        assert!(
            read(&c.bytes(&bases[c.base])).is_ok(),
            "{}: the parsers refuse it now, so the case moves to Parse",
            c.name
        );
    }
}

#[test]
fn the_cases_for_later_issues_are_well_formed_files_that_only_the_missing_feature_can_judge() {
    let (bases, cases) = corpus();
    let pending: Vec<&Case> = cases.iter().filter(|c| c.requires == Requires::Integrity).collect();
    assert!(pending.len() >= 3);
    for c in pending {
        // a parser sees nothing wrong with them: a digest that does not match is invisible without P4.6
        let bytes = c.bytes(&bases[c.base]);
        assert!(read(&bytes).is_ok(), "{}: only an integrity check may refuse this file", c.name);
        assert!(matches!(&c.expect, Expect::Refuse(r) if r[0].starts_with("INTEGRITY-")), "{}", c.name);
    }
}

/// Reference rules no case can reach, and why. Everything else the reference defines is pinned by a case.
const DEFENSIVE: &[(&str, &str)] = &[
    (
        "LAYOUT-05",
        "a block or the directory outside the file: the header, the manifest pointer and parse_directory_checked refuse every such file first",
    ),
    (
        "LAYOUT-07",
        "a block that overlaps the one before it or runs into the directory: DIR-08 and DIR-11 refuse it first",
    ),
    (
        "NEGOTIATE-01",
        "pinned by a_reader_that_claims_negotiation_must_accept_the_optional_block_it_does_not_know",
    ),
];

#[test]
fn every_layout_consistency_and_limit_rule_of_the_reference_is_pinned_by_a_case_or_declared_unreachable() {
    let (_, cases) = corpus();
    let pinned: BTreeSet<&str> = cases
        .iter()
        .filter_map(|c| if let Expect::Refuse(r) = &c.expect { Some(r.iter().copied()) } else { None })
        .flatten()
        .collect();
    let source = include_str!("../src/reference.rs");
    let mut defined = BTreeSet::new();
    for prefix in ["LAYOUT-", "CONSIST-", "LIMIT-", "NEGOTIATE-"] {
        for (at, _) in source.match_indices(&format!("\"{prefix}")) {
            defined.insert(source[at + 1..at + 1 + prefix.len() + 2].to_string());
        }
    }
    assert!(defined.len() >= 14, "{defined:?}");
    for rule in &defined {
        let declared = DEFENSIVE.iter().any(|(r, _)| r == rule);
        assert!(
            pinned.contains(rule.as_str()) != declared,
            "{rule}: pinned by a case = {}, declared unreachable = {declared}; it has to be exactly one",
            pinned.contains(rule.as_str())
        );
    }
}

#[test]
fn a_sweep_case_wants_a_rule_the_container_knows_not_just_any_refusal() {
    let (_, cases) = corpus();
    let first =
        |prefix: &str| cases.iter().find(|c| c.name.starts_with(prefix)).expect("a sweep case").name.clone();
    let names = [first("truncation/prefix/drawing/"), first("truncation/remove_byte/drawing/")];
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let report = run_subset(WrongRule, &names, Limits::default());
    assert_eq!(report.failures.len(), 2, "{}", report.summary());
    assert!(
        report.failures.iter().all(|(_, f)| matches!(f, Failure::WrongRule { got, .. } if got == "XX-00"))
    );
}

/// Allocates 64 KiB on every read: little, but 60 times what the reference ever needs for a small file.
struct AllocatesSixtyFourKiB;
impl Reader for AllocatesSixtyFourKiB {
    fn name(&self) -> String {
        "allocates 64 KiB".into()
    }
    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse]
    }
    fn read(&self, bytes: &[u8]) -> Verdict {
        let v = vec![1u8; 64 << 10];
        std::hint::black_box(&v);
        ReferenceReader::new().read(bytes)
    }
}

#[test]
fn the_allocation_floor_is_calibrated_to_the_reference_and_catches_a_small_waste() {
    let limits = Limits::default();
    assert!(limits.alloc_floor <= 16 << 10, "the floor is {}", limits.alloc_floor);
    let ok = run_full(ReferenceReader::new(), limits);
    assert!(ok.passed(), "{}", ok.summary());
    assert!(
        ok.peak_allocation < limits.alloc_floor,
        "calibration: the reference peaked at {}",
        ok.peak_allocation
    );
    let report = run_subset(AllocatesSixtyFourKiB, &["offsets/block_offset_past_eof"], limits);
    assert_eq!(report.failures.len(), 1, "{}", report.summary());
    assert!(matches!(report.failures[0].1, Failure::Allocated { .. }));
}
