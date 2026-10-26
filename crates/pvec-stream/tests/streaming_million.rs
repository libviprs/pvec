//! The million-entity streaming and load-shed harness (issue P5.8, pvec#17).
//!
//! Three deterministic 1,000,000-entity artifacts (mixed density, spline-heavy, straddler-heavy) and
//! their 100,000-entity siblings, each opened through the real reader and driven the way the runtime
//! drives a viewport: plan, read the planned blocks, read the straddler payloads and the advanced set the
//! viewport touches. Every measured quantity has a threshold below, and exceeding it fails the run.
//!
//! The panel's decision on the spline-heavy fixture (Q11): the advanced curve set is one set of six
//! blocks (STREAMING 2.8) and `ContainerReader::advanced()` reads all of it, so on the spline-heavy
//! fixture the resident bytes carry one term that grows with N. The memory criterion is therefore
//! asserted as `resident - advanced_set_bytes` for all three fixtures, with `advanced_set_bytes`
//! recorded per fixture in the results file, until pvec#148 (chunk the advanced set) removes the term
//! and the plain criterion returns. Mixed and straddler-heavy have no advanced set, so nothing is subtracted from what they pin.
//!
//! The cancellation claim is two numbers, not one: `shed_rate = units_discarded / units_superseded`
//! (superseded work is dropped rather than applied) and `useful_fraction = 1 - units_discarded /
//! units_started` (the runtime does not thrash). Both are in the results file with the units per
//! viewport, because the second depends on it.

#[path = "streaming_million/harness.rs"]
mod harness;

use harness::{Kind, MILLION, SMALL, Scale};
use pvec_container::source::BlockSource;
use std::time::Duration;

// ---------------------------------------------------------------- the thresholds

/// Directory ready (open, spatial directory, straddler index) on a debug build, in seconds. A wall-clock
/// bound has a wide tolerance: it catches a reader that starts scanning the file, not a slow CI host.
const MAX_DIRECTORY_READY_SECS: f64 = 5.0;
/// Time to the first useful drawing (complete Tier 0 for the viewport), debug build, seconds.
const MAX_FIRST_DRAWING_SECS: f64 = 30.0;
/// Range requests for complete Tier 0 of a viewport, per scale (wide, mid, tight), besides the advanced
/// chunks' own ranges (one per chunk since #153, so they grow with the curves and not with the viewport).
/// The old single-set read was one more range, so each cap still has that one in it.
const MAX_RANGES: [u64; 3] = [16, 12, 10];
/// Bytes transferred for a viewport besides the advanced set, per scale (wide, mid, tight).
const MAX_BYTES: [u64; 3] = [4 << 20, 1 << 20, 256 << 10];
/// Decode work (bytes stored plus bytes decoded) besides the advanced set, per scale (wide, mid, tight).
const MAX_DECODE_WORK: [u64; 3] = [8 << 20, 2 << 20, 512 << 10];
/// Peak resident bytes of one viewport on the 1,000,000-entity artifacts, the advanced set taken out
/// (the directory and the straddler index are in it).
const MAX_RESIDENT: u64 = 16 << 20;
/// The resident indexes (directory block, its parsed copy, straddler index) per entity. They are the
/// one structure a viewport cannot avoid reading whole, and the budget is per entity so that it is a
/// slope and not a size.
const MAX_INDEX_BYTES_PER_ENTITY: f64 = 6.0;
/// The advanced set as stored, per advanced record. Until pvec#148 the whole set is read for any viewport
/// that touches it, so this is the cost of that read per spline.
const MAX_ADVANCED_BYTES_PER_RECORD: f64 = 160.0;
/// How much the bytes a viewport pins (`held`) may differ between 100,000 and 1,000,000 entities: the
/// viewport is the same size in both, so about the same blocks are read. Measured worst case +23% (the
/// cells at the viewport's edge fall differently in a larger artifact); loading the file would be 10x.
const RESIDENT_TOLERANCE: f64 = 0.35;
/// Added to that for the small viewports, where one more block at the edge (about 3 KB) is tens of percent.
const RESIDENT_SLACK: f64 = 65536.0;

/// The superseded work the runtime drops, as a fraction of the superseded work.
const MIN_SHED_RATE: f64 = 0.95;
/// One minus the discarded fraction of the work started. The panel's prior was 0.5. Measured 0.91 at 74
/// units per viewport and about 11 units started before each cancel (8 ms per range, a change every
/// 100 ms): the one unit in flight is thrown away per job, so the fraction is 1 - 1/units_per_job. The
/// floor is 0.8, which a host that sleeps 25% long still clears.
const MIN_USEFUL_FRACTION: f64 = 0.8;
/// Viewport changes in the load-shed scenario, and its length in seconds.
const CHANGES: u32 = 100;
const SCENARIO_SECS: u64 = 10;

const FOLLOW_UP: &str = "pvec#148";

fn scales() -> [Scale; 3] {
    [Scale::Wide, Scale::Mid, Scale::Tight]
}

// ---------------------------------------------------------------- the generator

#[test]
fn the_generator_is_deterministic_for_every_fixture() {
    for kind in Kind::ALL {
        let a = harness::generate(kind, 20_000, 7);
        let b = harness::generate(kind, 20_000, 7);
        assert!(a == b, "{}: the same seed gives a byte-identical artifact", kind.name());
        let c = harness::generate(kind, 20_000, 8);
        assert!(a != c, "{}: another seed gives another artifact", kind.name());
    }
}

#[test]
fn the_fixtures_have_the_entities_and_the_shape_they_claim() {
    for kind in Kind::ALL {
        for n in [SMALL, MILLION] {
            let c = harness::counts(&harness::artifact(kind, n));
            assert_eq!(c.entities, u64::from(n), "{} {n}", kind.name());
            match kind {
                Kind::Mixed => assert_eq!((c.advanced, c.long), (0, 0)),
                Kind::SplineHeavy => {
                    assert!(c.advanced * 4 >= c.entities, "{} splines of {}", c.advanced, c.entities)
                }
                Kind::StraddlerHeavy => {
                    assert_eq!(c.advanced, 0);
                    assert!(c.long * 200 >= c.entities, "{} long of {}", c.long, c.entities);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the budgets

#[test]
fn every_measured_quantity_is_under_its_threshold_on_the_million_entity_artifacts() {
    for kind in Kind::ALL {
        let c = harness::counts(&harness::artifact(kind, MILLION));
        for (i, scale) in scales().into_iter().enumerate() {
            let m = harness::measure(kind, MILLION, scale);
            eprintln!("{} {}: {}", kind.name(), scale.name(), m.line());
            let name = format!("{} {}", kind.name(), scale.name());
            assert!(
                m.directory_ready.as_secs_f64() <= MAX_DIRECTORY_READY_SECS,
                "{name}: directory ready {:?}",
                m.directory_ready
            );
            assert!(
                m.first_drawing.as_secs_f64() <= MAX_FIRST_DRAWING_SECS,
                "{name}: first drawing {:?}",
                m.first_drawing
            );
            assert!(
                m.ranges_without_advanced() + u64::from(m.advanced_ranges > 0) <= MAX_RANGES[i],
                "{name}: {} ranges",
                m.ranges_without_advanced()
            );
            assert!(
                m.bytes_without_advanced() <= MAX_BYTES[i],
                "{name}: {} bytes",
                m.bytes_without_advanced()
            );
            assert!(
                m.work_without_advanced() <= MAX_DECODE_WORK[i],
                "{name}: {} work",
                m.work_without_advanced()
            );
            assert!(
                m.resident_without_advanced() <= MAX_RESIDENT,
                "{name}: {} resident",
                m.resident_without_advanced()
            );
            assert!(
                m.index_bytes as f64 <= MAX_INDEX_BYTES_PER_ENTITY * f64::from(MILLION),
                "{name}: {} index bytes",
                m.index_bytes
            );
            if c.advanced > 0 {
                assert!(
                    m.advanced_stored as f64 <= MAX_ADVANCED_BYTES_PER_RECORD * c.advanced as f64,
                    "{name}: {} advanced bytes",
                    m.advanced_stored
                );
            }
            assert!(m.blocks > 0, "{name}: the viewport drew something");
        }
    }
}

#[test]
fn what_a_viewport_pins_does_not_grow_between_100k_and_1m_with_the_advanced_set_taken_out() {
    for kind in Kind::ALL {
        for scale in scales() {
            let small = harness::measure(kind, SMALL, scale);
            let big = harness::measure(kind, MILLION, scale);
            let (s, b) = (small.held as f64, big.held as f64);
            eprintln!(
                "{} {}: held {s} -> {b}; resident - advanced {} -> {}; advanced_set_bytes {} -> {} ({FOLLOW_UP})",
                kind.name(),
                scale.name(),
                small.resident_without_advanced(),
                big.resident_without_advanced(),
                small.advanced_set_bytes,
                big.advanced_set_bytes
            );
            assert!(
                b <= s * (1.0 + RESIDENT_TOLERANCE) + RESIDENT_SLACK,
                "{} {}: {s} -> {b} bytes",
                kind.name(),
                scale.name()
            );
        }
    }
}

#[test]
fn mixed_and_straddler_heavy_have_no_advanced_set_so_what_they_pin_is_compared_with_nothing_subtracted() {
    for kind in [Kind::Mixed, Kind::StraddlerHeavy] {
        for scale in scales() {
            let small = harness::measure(kind, SMALL, scale);
            let big = harness::measure(kind, MILLION, scale);
            for m in [&small, &big] {
                assert_eq!(m.advanced_set_bytes, 0, "{} has no advanced set", kind.name());
                assert_eq!(m.resident_without_advanced(), m.peak_resident);
            }
            // `held` is already free of the indexes and of the advanced set. The reader's peak is not
            // compared between the two sizes: the open reads the directory through a buffer that is
            // about 1.2 bytes per entity beyond the index, and that transient is the peak of a small
            // viewport (mixed tight: 77,584 bytes above the index at 100k, 774,352 at 1M).
            let (s, b) = (small.held as f64, big.held as f64);
            assert!(
                b <= s * (1.0 + RESIDENT_TOLERANCE) + RESIDENT_SLACK,
                "{} {}: held {s} -> {b} bytes",
                kind.name(),
                scale.name()
            );
        }
    }
}

// ---------------------------------------------------------------- the load-shed scenario

#[test]
fn a_hundred_viewport_changes_shed_superseded_work_and_end_on_the_newest_viewport() {
    let r = harness::load_shed(CHANGES, SCENARIO_SECS);
    eprintln!("{}", r.line());
    assert_load_shed(&r);
}

/// What the scenario has to show, whichever source it ran against.
fn assert_load_shed(r: &harness::Shed) {
    assert_eq!(r.changes, CHANGES);
    assert!(
        r.jobs_cancelled * 10 >= u64::from(CHANGES) * 9,
        "the throttle makes nearly every job outlive its view: {}",
        r.jobs_cancelled
    );
    assert!(r.shed_rate() >= MIN_SHED_RATE, "shed_rate {}", r.shed_rate());
    assert!(r.useful_fraction() >= MIN_USEFUL_FRACTION, "useful_fraction {}", r.useful_fraction());
    assert!(
        r.units_discarded <= u64::from(CHANGES) * u64::from(pvec_stream::MAX_UNITS_AFTER_CANCEL),
        "{} units finished after a cancel",
        r.units_discarded
    );
    assert!(
        r.units_started * 2 <= r.units_planned,
        "superseded work is shed, not queued: {} of {} started",
        r.units_started,
        r.units_planned
    );
    assert!(r.final_matches_last_viewport, "the scene is the last viewport's and nothing else");
    assert_eq!(r.stale_applied, 0, "no result of an older generation reached the scene");
}

// ---------------------------------------------------------------- the hermetic range source (pvec#158)

fn container_exists(name: &str) -> bool {
    let ps = std::process::Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output()
        .expect("docker ps");
    !String::from_utf8_lossy(&ps.stdout).trim().is_empty()
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn the_throttle_runs_in_a_hermetic_container_that_serves_the_artifact_bytes() {
    let want = harness::artifact(Kind::Mixed, SMALL);
    let name = harness::with_hermetic_source(&want, Duration::from_millis(8), 100 << 20, |source, name| {
        assert_eq!(source.len(), want.len() as u64);
        for (offset, len) in [(0u64, 64u32), (4096, 1000), (want.len() as u64 - 17, 17)] {
            let got = source.read_range(offset, len).expect("a range");
            assert_eq!(got.as_slice(), &want[offset as usize..offset as usize + len as usize]);
        }
        assert!(source.read_range(want.len() as u64 - 4, 8).is_err(), "a range past the end is refused");
        name.to_string()
    });
    if let Some(name) = name {
        assert!(!container_exists(&name), "{name} is still there after a pass");
    }
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn the_container_is_removed_when_the_scenario_fails() {
    let name = std::sync::Mutex::new(String::new());
    let bytes = harness::artifact(Kind::Mixed, SMALL);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness::with_hermetic_source(&bytes, Duration::ZERO, 1 << 30, |_, n| {
            *name.lock().unwrap() = n.to_string();
            panic!("the scenario failed");
        })
    }));
    assert!(r.is_err());
    let name = name.into_inner().unwrap();
    assert!(!name.is_empty(), "the container never started");
    assert!(!container_exists(&name), "{name} is still there after a failure");
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn a_hundred_viewport_changes_shed_superseded_work_against_the_source_in_the_container() {
    let Some(r) = harness::load_shed_hermetic(CHANGES, SCENARIO_SECS) else { return };
    eprintln!("hermetic: {}", r.line());
    assert_load_shed(&r);
}

#[test]
fn the_results_file_says_the_range_source_is_in_a_hermetic_container_and_no_longer_a_gap() {
    assert!(RESULTS.contains("hermetic container"), "the results file never says the source is hermetic");
    assert!(!RESULTS.contains("is in process and deterministic"), "the in-process gap is still described");
    assert!(!RESULTS.contains("a gap, filed as a follow-up (pvec#158)"));
}

// ---------------------------------------------------------------- the results file

const RESULTS: &str = include_str!("streaming_million_results.md");

#[test]
fn the_committed_results_file_has_every_number_and_says_where_it_ran() {
    for kind in Kind::ALL {
        for n in [SMALL, MILLION] {
            for scale in scales() {
                let row = format!("| {} | {} | {} |", kind.name(), n, scale.name());
                assert!(RESULTS.contains(&row), "no row {row}");
            }
        }
    }
    for needle in [
        "advanced_set_bytes",
        "shed_rate",
        "useful_fraction",
        "units per viewport",
        FOLLOW_UP,
        "STREAMING 2.8",
        "native x86_64",
        "arm64",
    ] {
        assert!(RESULTS.contains(needle), "the results file never says {needle:?}");
    }
}

/// Regenerates the results file: `PVEC_RESULTS_OUT=<path> cargo test --release -- --ignored write_results`.
#[test]
#[ignore = "writes the results file; run it on the host the numbers are for"]
fn write_results() {
    // The container stage runs every ignored test; without a target there is nothing to write.
    let Ok(out) = std::env::var("PVEC_RESULTS_OUT") else {
        eprintln!("PVEC_RESULTS_OUT is not set: nothing written");
        return;
    };
    std::fs::write(out, harness::results_markdown(CHANGES, SCENARIO_SECS)).expect("written");
}
