//! Parser and resource security limits (issue P4.8).
//!
//! One named test per row of the issue's table. Each one builds a valid artifact with the real writer,
//! changes a few bytes so that it lies about a count, a length or an offset, and asserts three things:
//! the refusal is a `LimitExceeded` (or, where the issue says so, the parser's own rule), it carries the
//! declared value and the ceiling, and a counting allocator saw that the size the file declared was
//! never requested. A second group shows that the ceilings are configuration and not hard-coded
//! refusals: raising one lets a fixture through that the lower value refused.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
mod support;

use pvec_container::blocks::arena::ArenaError;
use pvec_container::blocks::hot::HOT_HEADER_FIELDS;
use pvec_container::limits::{Accountant, LimitExceeded, LimitKind, Limits};
use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig, Validated};
use pvec_container::source::MemorySource;
use std::sync::Arc;
use support::art::{self, Art};

#[global_allocator]
static A: counting_alloc::Counting = counting_alloc::Counting;

type Reader = ContainerReader<MemorySource<Vec<u8>>>;

fn config(limits: Limits) -> ReaderConfig {
    ReaderConfig { limits, ..ReaderConfig::default() }
}

/// Opens and fully validates, returning what was requested from the allocator while doing it.
fn read(bytes: &[u8], limits: Limits) -> (Result<Validated, ReadError>, usize) {
    let source = MemorySource::new(bytes.to_vec());
    let before = counting_alloc::requested();
    let result = ContainerReader::open(source, config(limits)).and_then(|r| r.validate());
    (result, counting_alloc::requested() - before)
}

fn limit_of(result: Result<Validated, ReadError>) -> LimitExceeded {
    match result {
        Err(ReadError::Limit(l)) => l,
        other => panic!("expected a LimitExceeded, got {other:?}"),
    }
}

/// Far less than any size these tests have a file declare (the smallest is 1 MiB).
const SMALL: usize = 256 << 10;

fn hot_header(art: &Art, block: usize, name: &str) -> usize {
    let f = HOT_HEADER_FIELDS.iter().find(|f| f.name == name).expect("a hot header field");
    art.entries[block].offset as usize + f.offset
}

// ---------------------------------------------------------------- the table of the issue

#[test]
fn entity_counts_a_block_header_declaring_4294967295_entities_in_a_4kib_artifact() {
    let art = art::lines(8, 1);
    assert!(art.len() <= 4096, "the artifact is {} bytes", art.len());
    let at = hot_header(&art, 0, "entity_count");
    let hostile = art.with_u32(at, u32::MAX);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::EntitiesPerBlock);
    assert_eq!(l.declared, u64::from(u32::MAX));
    assert_eq!(l.ceiling, u64::from(Limits::DEFAULT.max_entities_per_block));
    assert!(used < SMALL, "a refusal that requested {used} bytes reserved room for what the file declared");
}

#[test]
fn entity_counts_a_directory_declaring_4294967295_entities_is_refused_when_the_file_is_opened() {
    let art = art::lines(8, 1);
    let at = art.entry_field(0, "count");
    let hostile = art.with_u32(at, u32::MAX);
    let source = MemorySource::new(hostile.bytes.clone());
    let before = counting_alloc::requested();
    let err = ContainerReader::open(source, ReaderConfig::default()).err().expect("refused at open");
    let used = counting_alloc::requested() - before;
    let l = *err.limit().expect("a LimitExceeded");
    assert_eq!(
        l.limit,
        LimitKind::EntitiesPerBlock,
        "the block ceiling is the more specific one and is checked first"
    );
    assert_eq!(l.declared, u64::from(u32::MAX));
    assert_eq!(l.ceiling, u64::from(Limits::DEFAULT.max_entities_per_block));
    assert!(used < SMALL, "{used} bytes requested");
}

#[test]
fn offsets_and_lengths_a_descriptor_whose_end_passes_the_artifact_is_a_limit() {
    let art = art::lines(8, 1);
    let len = art.len() as u64;
    let offset = (len - 8) & !7;
    let stored = u64::from(art.entries[0].stored_length);
    let hostile = art.clone().with_u64(art.entry_field(0, "offset"), offset);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::Extent);
    assert_eq!(l.declared, offset + stored);
    assert_eq!(l.ceiling, len);
    assert!(used < SMALL);
}

#[test]
fn offsets_and_lengths_a_descriptor_whose_end_overflows_u64_is_a_limit() {
    let art = art::lines(8, 1);
    let len = art.len() as u64;
    let hostile = art.clone().with_u64(art.entry_field(0, "offset"), u64::MAX - 7);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::Extent);
    assert_eq!(l.declared, u64::MAX, "an end that overflows is reported as the largest value");
    assert_eq!(l.ceiling, len);
    assert!(used < SMALL);
}

#[test]
fn nesting_depth_a_composite_chain_deeper_than_the_ceiling_is_a_limit() {
    let art = art::composites(4);
    let browser_like = Limits { max_nesting_depth: 3, ..Limits::DEFAULT };
    let (result, used) = read(&art.bytes, browser_like);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::NestingDepth);
    assert_eq!((l.declared, l.ceiling), (4, 3), "the first level over the ceiling, and the ceiling");
    assert!(used < SMALL);
    // at NP-18's own maximum the same chain is fine
    assert!(read(&art.bytes, Limits::DEFAULT).0.is_ok());
}

#[test]
fn nesting_depth_a_self_referential_cycle_is_refused_as_a_cycle() {
    let art = art::composites(2);
    // record 0 is the spline (id 10), record 1 the composite (id 22); its one segment names record 0
    let segs = art.index_of(3, 4);
    let block = art.entries[segs].offset as usize;
    let entries = art.u32_at(block + 8) as usize;
    let first_element = block + 32 + 8 * entries;
    assert_eq!(art.u32_at(first_element), 0, "the composite names the spline");
    let hostile = art.with_u32(first_element, 1);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    match result {
        Err(e @ ReadError::Arena(ArenaError::CompositeCycle { .. })) => assert_eq!(e.rule(), "ARN-21"),
        other => panic!("expected a cycle, got {other:?}"),
    }
    assert!(used < SMALL);
}

#[test]
fn decompression_expansion_ratio_a_block_declaring_4gib_from_1kib_is_a_limit() {
    let art = art::lines(24, 1);
    let stored = u64::from(art.entries[0].stored_length);
    assert!((900..=1100).contains(&stored), "the block is {stored} bytes, about 1 KiB");
    let hostile = art.clone().with_u32(art.entry_field(0, "uncompressed_length"), u32::MAX);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::ExpansionRatio);
    assert_eq!(l.declared, u64::from(u32::MAX));
    assert_eq!(l.ceiling, stored * u64::from(Limits::DEFAULT.max_expansion_ratio));
    assert!(used < SMALL, "{used} bytes requested for a block declared as 4 GiB");
}

#[test]
fn knot_ceiling_a_knot_count_above_the_ceiling_is_a_limit() {
    let art = art::splines();
    let (result, used) = read(&art.bytes, Limits { max_knots: 12, ..Limits::DEFAULT });
    let l = limit_of(result);
    assert_eq!((l.limit, l.declared, l.ceiling), (LimitKind::Knots, 14, 12));
    assert!(used < SMALL);
}

#[test]
fn knot_ceiling_a_knot_count_that_lies_is_a_limit_before_the_range_check() {
    let art = art::splines();
    let knots = art.index_of(3, 1);
    let entry = art.entries[knots].offset as usize + 32;
    let hostile = art.with_u32(entry + 4, u32::MAX);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::Knots);
    assert_eq!(l.declared, u64::from(u32::MAX));
    assert_eq!(l.ceiling, u64::from(Limits::DEFAULT.max_knots));
    assert!(used < SMALL);
}

#[test]
fn knot_ceiling_a_knot_count_inconsistent_with_degree_and_control_points_is_refused() {
    let art = art::splines();
    let knots = art.index_of(3, 1);
    let entry = art.entries[knots].offset as usize + 32;
    assert_eq!(art.u32_at(entry + 4), 14);
    let hostile = art.with_u32(entry + 4, 13);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    assert_eq!(result.expect_err("refused").rule(), "ARN-13");
    assert!(used < SMALL);
}

#[test]
fn control_point_ceiling_a_control_point_count_above_the_ceiling_is_a_limit() {
    let art = art::splines();
    let (result, used) = read(&art.bytes, Limits { max_control_points: 8, ..Limits::DEFAULT });
    let l = limit_of(result);
    assert_eq!((l.limit, l.declared, l.ceiling), (LimitKind::ControlPoints, 10, 8));
    assert!(used < SMALL);
}

#[test]
fn control_point_ceiling_a_count_of_4294967295_is_a_limit_and_is_not_allocated() {
    let art = art::splines();
    let cps = art.index_of(3, 3);
    let entry = art.entries[cps].offset as usize + 32;
    let hostile = art.with_u32(entry + 4, u32::MAX);
    let (result, used) = read(&hostile.bytes, Limits::DEFAULT);
    let l = limit_of(result);
    assert_eq!(l.limit, LimitKind::ControlPoints);
    assert_eq!(l.declared, u64::from(u32::MAX));
    assert_eq!(l.ceiling, u64::from(Limits::DEFAULT.max_control_points));
    assert!(used < SMALL, "{used} bytes requested for 2^32 control points");
}

#[test]
fn allocation_budget_many_legal_blocks_whose_live_total_crosses_the_budget_are_refused() {
    let art = art::lines(500, 12);
    let blocks = art.entries.len();
    assert_eq!(blocks, 12);
    let one = u64::from(art.entries[0].uncompressed_length);
    let budget = 3 * one;
    let limits = Limits { max_alloc_bytes: budget, ..Limits::DEFAULT };
    let cfg = ReaderConfig { limits, cache_bytes: 1 << 30, ..ReaderConfig::default() };
    let reader: Reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), cfg).expect("opens");
    let mut held = Vec::new();
    let mut refusal = None;
    let mut used_by_refusal = 0;
    for i in 0..blocks {
        let before = counting_alloc::requested();
        match reader.read_block(BlockId(i as u32)) {
            Ok(b) => held.push(b),
            Err(e) => {
                used_by_refusal = counting_alloc::requested() - before;
                refusal = Some(e);
                break;
            }
        }
    }
    let l = *refusal.expect("the budget refuses a block").limit().expect("a LimitExceeded");
    assert_eq!(l.limit, LimitKind::AllocationBudget);
    assert_eq!(l.ceiling, budget);
    assert!(l.declared > budget, "declared {} is the total the read would have reached", l.declared);
    assert!(!held.is_empty() && held.len() < blocks, "{} blocks fit", held.len());
    assert!(reader.accountant().peak() <= budget, "the live total never passed the budget");
    assert!(
        used_by_refusal < one as usize / 2,
        "the refused block's {one} bytes were requested ({used_by_refusal} bytes)"
    );
    // the reader is still usable: letting go makes room
    drop(held);
    assert!(reader.read_block(BlockId(0)).is_ok());
}

#[test]
fn allocation_budget_the_cache_gives_way_to_a_block_the_budget_would_refuse() {
    let art = art::lines(500, 12);
    let one = u64::from(art.entries[0].uncompressed_length);
    let limits = Limits { max_alloc_bytes: 3 * one, ..Limits::DEFAULT };
    let cfg = ReaderConfig { limits, cache_bytes: 1 << 30, ..ReaderConfig::default() };
    let reader: Reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), cfg).expect("opens");
    for i in 0..art.entries.len() {
        // each block is dropped at once, so only the cache holds it, and the cache is the first to go
        reader.read_block(BlockId(i as u32)).expect("room is made by evicting");
    }
    assert!(reader.stats().evictions > 0);
    assert!(reader.accountant().peak() <= 3 * one);
}

// ---------------------------------------------------------------- the ceilings are configuration

/// Parses with `low` and is refused with `kind`, parses with `high` and is accepted.
fn raising_lets_it_through(bytes: &[u8], low: Limits, kind: LimitKind, high: Limits) {
    let l = limit_of(read(bytes, low).0);
    assert_eq!(l.limit, kind, "{l:?}");
    let accepted = read(bytes, high).0;
    assert!(accepted.is_ok(), "raising {kind:?} should let the fixture through, got {accepted:?}");
}

#[test]
fn raising_a_ceiling_in_configuration_makes_a_refused_fixture_parse() {
    let d = Limits::DEFAULT;
    let sp = art::splines();
    raising_lets_it_through(
        &sp.bytes,
        Limits { max_control_points: 8, ..d },
        LimitKind::ControlPoints,
        Limits { max_control_points: 10, ..d },
    );
    raising_lets_it_through(
        &sp.bytes,
        Limits { max_knots: 12, ..d },
        LimitKind::Knots,
        Limits { max_knots: 14, ..d },
    );
    let nested = art::composites(4);
    raising_lets_it_through(
        &nested.bytes,
        Limits { max_nesting_depth: 3, ..d },
        LimitKind::NestingDepth,
        Limits { max_nesting_depth: 4, ..d },
    );
    let ln = art::lines(8, 1);
    raising_lets_it_through(
        &ln.bytes,
        Limits { max_entities_per_block: 4, ..d },
        LimitKind::EntitiesPerBlock,
        Limits { max_entities_per_block: 8, ..d },
    );
    raising_lets_it_through(
        &ln.bytes,
        Limits { max_entities_per_artifact: 4, ..d },
        LimitKind::EntitiesPerArtifact,
        Limits { max_entities_per_artifact: 8, ..d },
    );
    let stored = ln.entries[0].stored_length;
    raising_lets_it_through(
        &ln.bytes,
        Limits { max_block_bytes: stored - 8, ..d },
        LimitKind::BlockBytes,
        Limits { max_block_bytes: stored, ..d },
    );
    raising_lets_it_through(
        &ln.bytes,
        Limits { max_artifact_bytes: ln.len() as u64 - 1, ..d },
        LimitKind::ArtifactBytes,
        Limits { max_artifact_bytes: ln.len() as u64, ..d },
    );
    let two = art::lines(4, 2);
    raising_lets_it_through(
        &two.bytes,
        Limits { max_blocks: 1, ..d },
        LimitKind::Blocks,
        Limits { max_blocks: 2, ..d },
    );
    raising_lets_it_through(
        &ln.bytes,
        Limits { max_alloc_bytes: 100, ..d },
        LimitKind::AllocationBudget,
        Limits { max_alloc_bytes: 1 << 20, ..d },
    );
    raising_lets_it_through(
        &ln.bytes,
        Limits { max_decode_work: 100, ..d },
        LimitKind::DecodeWork,
        Limits { max_decode_work: 1 << 20, ..d },
    );
}

#[test]
fn raising_the_ratio_is_what_admits_a_block_that_declares_more_than_it_stores() {
    // with no codec the lengths must be equal (DIR-09), so the ratio can only be shown to be a
    // configuration value by the refusal moving: at 64 it is the limit, at a ratio the lie fits in it
    // is the container's own rule
    let art = art::lines(24, 1);
    let stored = art.entries[0].stored_length;
    let hostile = art.clone().with_u32(art.entry_field(0, "uncompressed_length"), stored * 65);
    assert_eq!(limit_of(read(&hostile.bytes, Limits::DEFAULT).0).limit, LimitKind::ExpansionRatio);
    let roomy = Limits { max_expansion_ratio: 100, ..Limits::DEFAULT };
    assert_eq!(read(&hostile.bytes, roomy).0.expect_err("still a lie").rule(), "DIR-09");
}

#[test]
fn a_ceiling_above_np_18_is_refused_at_open_and_not_clamped() {
    let art = art::lines(4, 1);
    let too_big = Limits { max_control_points: Limits::DEFAULT.max_control_points + 1, ..Limits::DEFAULT };
    let err = ContainerReader::open(MemorySource::new(art.bytes), config(too_big)).err().expect("refused");
    assert!(matches!(err, ReadError::BadLimits(_)), "{err:?}");
}

#[test]
fn the_browser_preset_is_stricter_and_refuses_what_the_defaults_admit() {
    let art = art::composites(4);
    assert!(read(&art.bytes, Limits::DEFAULT).0.is_ok());
    let l = limit_of(read(&art.bytes, Limits::BROWSER).0);
    assert_eq!(l.limit, LimitKind::NestingDepth);
    for kind in LimitKind::ALL {
        let (d, b) = (Limits::DEFAULT.get(kind), Limits::BROWSER.get(kind));
        assert!(b <= d, "{kind:?}: the browser preset ({b}) is above the default ({d})");
    }
}

#[test]
fn every_row_of_the_issue_table_has_its_named_test() {
    let source = include_str!("issue_9_limits.rs");
    for row in [
        "entity_counts_",
        "offsets_and_lengths_",
        "nesting_depth_",
        "decompression_expansion_ratio_",
        "knot_ceiling_",
        "control_point_ceiling_",
        "allocation_budget_",
    ] {
        assert!(source.contains(&format!("fn {row}")), "no test for the row {row}");
    }
}

#[test]
fn the_accountant_is_shared_by_clones_and_counts_what_is_live() {
    let a = Accountant::new(&Limits { max_alloc_bytes: 10, ..Limits::DEFAULT });
    let b = a.clone();
    let _held = a.reserve(8).unwrap();
    assert_eq!(b.reserve(3).unwrap_err().declared, 11);
    let _arc = Arc::new(a);
}
