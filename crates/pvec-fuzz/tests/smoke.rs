//! Issue P7.8: the smoke gate. Each target runs for a number of executions from its seeds, mutated, and any
//! panic, hang or execution over the memory budget is a finding. `PVEC_FUZZ_EXECUTIONS` sets the number (the
//! default keeps `cargo test` quick); the counts are printed so a run can be quoted.
#![allow(unsafe_code)]

mod support;

use pvec_container::Limits;
use pvec_fuzz::TARGETS;
use pvec_fuzz::seeds::{corpus, for_target};
use pvec_fuzz::smoke::{Finding, run};
use support::*;

#[global_allocator]
static ALLOCATOR: counting_alloc::Counting = counting_alloc::Counting;

fn executions() -> u64 {
    std::env::var("PVEC_FUZZ_EXECUTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000)
}

#[test]
fn every_target_runs_its_smoke_budget_with_no_finding() {
    let seeds = corpus();
    for (name, _) in TARGETS {
        let inputs = for_target(name, &seeds);
        let report = run(name, &inputs, executions(), 0x5eed, &metered);
        println!(
            "SMOKE {name} executions={} distinct_words={} findings={} words={:?}",
            report.executions,
            report.reached.len(),
            report.findings.len(),
            report.reached.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>()
        );
        assert_eq!(report.executions, executions(), "{name}");
        assert!(
            report.findings.is_empty(),
            "{name}: {:#?}",
            report
                .findings
                .iter()
                .map(|f| format!("{f:?}").chars().take(300).collect::<String>())
                .collect::<Vec<_>>()
        );
        assert!(report.reached.len() >= 2, "{name} only ever reached {:?}", report.reached);
    }
}

#[test]
fn an_execution_over_the_memory_budget_is_recorded_as_a_finding_and_not_tolerated_as_slow() {
    let inputs = vec![vec![1u8; 64]];
    let over = |f: &mut dyn FnMut()| {
        f();
        usize::MAX / 2
    };
    let report = run("fuzz_header", &inputs, 5, 1, &over);
    assert_eq!(report.findings.len(), 5);
    assert!(
        matches!(report.findings[0], Finding::Memory { budget, .. } if budget == Limits::DEFAULT.max_alloc_bytes as usize)
    );
}

#[test]
fn a_target_that_panics_is_recorded_with_its_input() {
    // the driver catches panics: a target name it does not know is a panic of the driver's own, so use a
    // real target with an input the driver is told to treat as the finding
    let report = pvec_fuzz::smoke::run_target(|_| panic!("boom"), &[vec![9]], 3, 1, &metered);
    assert_eq!(report.findings.len(), 3);
    assert!(matches!(&report.findings[0], Finding::Panic { message, .. } if message == "boom"));
}
