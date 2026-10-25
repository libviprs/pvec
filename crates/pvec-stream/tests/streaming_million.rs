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
//! and the plain criterion returns. Mixed and straddler-heavy assert the plain criterion already.
//!
//! The cancellation claim is two numbers, not one: `shed_rate = units_discarded / units_superseded`
//! (superseded work is dropped rather than applied) and `useful_fraction = 1 - units_discarded /
//! units_started` (the runtime does not thrash). Both are in the results file with the units per
//! viewport, because the second depends on it.

#[path = "streaming_million/harness.rs"]
mod harness;

use harness::{Kind, MILLION, SMALL, Scale};

// ---------------------------------------------------------------- the thresholds

/// Directory ready (open, spatial directory, straddler index) on a debug build, in seconds. A wall-clock
/// bound has a wide tolerance: it catches a reader that starts scanning the file, not a slow CI host.
const MAX_DIRECTORY_READY_SECS: f64 = 5.0;
/// Time to the first useful drawing (complete Tier 0 for the viewport), debug build, seconds.
const MAX_FIRST_DRAWING_SECS: f64 = 30.0;
/// Range requests for complete Tier 0 of a viewport, per scale (wide, mid, tight).
const MAX_RANGES: [u64; 3] = [64, 32, 16];
/// Bytes transferred for a viewport, per scale (wide, mid, tight).
const MAX_BYTES: [u64; 3] = [48 << 20, 16 << 20, 4 << 20];
/// Decode work (bytes stored plus bytes decoded) for a viewport, per scale (wide, mid, tight).
const MAX_DECODE_WORK: [u64; 3] = [256 << 20, 96 << 20, 24 << 20];
/// Peak resident bytes of one viewport, with the advanced set taken out, on the 1,000,000-entity
/// artifacts, per scale (wide, mid, tight).
const MAX_RESIDENT: [u64; 3] = [96 << 20, 32 << 20, 8 << 20];
/// How much `resident - advanced_set_bytes` may differ between 100,000 and 1,000,000 entities: the cache
/// and the planner are the runtime's, and the viewport is the same size in both.
const RESIDENT_TOLERANCE: f64 = 0.10;

/// The superseded work the runtime drops, as a fraction of the superseded work.
const MIN_SHED_RATE: f64 = 0.95;
/// One minus the discarded fraction of the work started. The panel's prior was 0.5; the measured value
/// and the units per viewport it depends on are in the results file.
const MIN_USEFUL_FRACTION: f64 = 0.5;
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
                Kind::SplineHeavy => assert!(c.advanced * 4 >= c.entities, "{} splines of {}", c.advanced, c.entities),
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
        for (i, scale) in scales().into_iter().enumerate() {
            let m = harness::measure(kind, MILLION, scale);
            eprintln!("{}", m.line());
            let name = format!("{} {}", kind.name(), scale.name());
            assert!(m.directory_ready.as_secs_f64() <= MAX_DIRECTORY_READY_SECS, "{name}: directory ready {:?}", m.directory_ready);
            assert!(m.first_drawing.as_secs_f64() <= MAX_FIRST_DRAWING_SECS, "{name}: first drawing {:?}", m.first_drawing);
            assert!(m.ranges <= MAX_RANGES[i], "{name}: {} ranges", m.ranges);
            assert!(m.bytes <= MAX_BYTES[i], "{name}: {} bytes", m.bytes);
            assert!(m.decode_work <= MAX_DECODE_WORK[i], "{name}: {} work", m.decode_work);
            assert!(m.resident_without_advanced() <= MAX_RESIDENT[i], "{name}: {} resident", m.resident_without_advanced());
            assert!(m.blocks > 0, "{name}: the viewport drew something");
        }
    }
}

#[test]
fn peak_resident_bytes_without_the_advanced_set_do_not_grow_between_100k_and_1m() {
    for kind in Kind::ALL {
        for scale in scales() {
            let small = harness::measure(kind, SMALL, scale);
            let big = harness::measure(kind, MILLION, scale);
            let (s, b) = (small.resident_without_advanced() as f64, big.resident_without_advanced() as f64);
            eprintln!(
                "{} {}: resident - advanced {s} -> {b}; advanced_set_bytes {} -> {}",
                kind.name(), scale.name(), small.advanced_set_bytes, big.advanced_set_bytes
            );
            assert!(b <= s * (1.0 + RESIDENT_TOLERANCE), "{} {}: {s} -> {b} bytes", kind.name(), scale.name());
        }
    }
}

#[test]
fn mixed_and_straddler_heavy_hold_the_plain_criterion_with_nothing_subtracted() {
    for kind in [Kind::Mixed, Kind::StraddlerHeavy] {
        for scale in scales() {
            let small = harness::measure(kind, SMALL, scale);
            let big = harness::measure(kind, MILLION, scale);
            assert_eq!((small.advanced_set_bytes, big.advanced_set_bytes), (0, 0), "{} has no advanced set", kind.name());
            assert!(
                big.peak_resident as f64 <= small.peak_resident as f64 * (1.0 + RESIDENT_TOLERANCE),
                "{} {}: {} -> {} bytes",
                kind.name(), scale.name(), small.peak_resident, big.peak_resident
            );
        }
    }
}

// ---------------------------------------------------------------- the load-shed scenario

#[test]
fn a_hundred_viewport_changes_shed_superseded_work_and_end_on_the_newest_viewport() {
    let r = harness::load_shed(CHANGES, SCENARIO_SECS);
    eprintln!("{}", r.line());
    assert_eq!(r.changes, CHANGES);
    assert!(r.jobs_cancelled * 10 >= u64::from(CHANGES) * 9, "the throttle makes nearly every job outlive its view: {}", r.jobs_cancelled);
    assert!(r.shed_rate() >= MIN_SHED_RATE, "shed_rate {}", r.shed_rate());
    assert!(r.useful_fraction() >= MIN_USEFUL_FRACTION, "useful_fraction {}", r.useful_fraction());
    assert!(
        r.units_discarded <= u64::from(CHANGES) * u64::from(pvec_stream::MAX_UNITS_AFTER_CANCEL),
        "{} units finished after a cancel",
        r.units_discarded
    );
    assert!(r.units_started * 2 <= r.units_planned, "superseded work is shed, not queued: {} of {} started", r.units_started, r.units_planned);
    assert!(r.final_matches_last_viewport, "the scene is the last viewport's and nothing else");
    assert_eq!(r.stale_applied, 0, "no result of an older generation reached the scene");
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
    let out = std::env::var("PVEC_RESULTS_OUT").expect("PVEC_RESULTS_OUT names the file to write");
    std::fs::write(out, harness::results_markdown(CHANGES, SCENARIO_SECS)).expect("written");
}
