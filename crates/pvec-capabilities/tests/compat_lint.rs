//! The registry lint (issue P7.6): a capability may not leave the registry without passing through
//! `deprecated` and `write-disabled`, and a breaking change may not land on a minor format version.

use pvec_capabilities::{
    Breaking, DEPRECATED_MIN_MINORS, FORMAT_VERSION, LintFinding, Registry, Snapshot, Status, Version,
    WRITE_DISABLED_MIN_MINORS, lint_transition,
};

/// A snapshot with one capability (id 7) and the given format version, status and retired list.
fn snap(
    format: &str,
    version: &str,
    requirement: &str,
    status: &str,
    since: &str,
    retired: &[u32],
) -> Snapshot {
    let mut t = format!("format_version = \"{format}\"\n");
    if !status.is_empty() {
        t.push_str(&format!(
            "\n[[capability]]\nid = 7\nname = \"seven\"\nclass = \"component\"\nversion = \"{version}\"\n\
             requirement = \"{requirement}\"\nmin_reader = \"1.0.0\"\nstatus = \"{status}\"\nstatus_since = \"{since}\"\n"
        ));
    }
    for id in retired {
        t.push_str(&format!("\n[[retired]]\nid = {id}\nname = \"gone\"\n"));
    }
    Snapshot::parse(&t).expect("a valid snapshot")
}

fn active(format: &str) -> Snapshot {
    snap(format, "1.0.0", "optional", "active", "1.0.0", &[])
}

#[test]
fn the_committed_policy_numbers_are_the_ones_the_document_states() {
    assert_eq!((DEPRECATED_MIN_MINORS, WRITE_DISABLED_MIN_MINORS), (2, 2));
    assert_eq!(FORMAT_VERSION, Version::new(1, 3, 0));
}

#[test]
fn the_v1_registry_round_trips_through_its_snapshot_text() {
    let s = Snapshot::from_registry(&Registry::v1());
    assert_eq!(s.entries.len(), 24);
    assert!(s.entries.iter().all(|e| e.status == Status::Active));
    // an entry carries the format version it entered at: 21 is the first 1.1.0 addition, 22 the first 1.2.0 one
    let since = |id: u32| match id {
        21 => Version::new(1, 1, 0),
        22 => Version::new(1, 2, 0),
        _ => Version::new(1, 0, 0),
    };
    assert!(s.entries.iter().all(|e| e.status_since == since(e.id)));
    let text = s.render();
    assert!(text.ends_with('\n') && !text.ends_with("\n\n"));
    assert_eq!(Snapshot::parse(&text).unwrap(), s);
    assert_eq!(lint_transition(&s, &s), vec![]);
}

#[test]
fn a_snapshot_with_a_malformed_line_is_refused_with_the_line() {
    for bad in [
        "format_version = 1.0.0\n",
        "id = 3\n",
        "[[capability]]\nid = x\n",
        "[[nope]]\n",
        "format_version = \"1.0\"\n",
    ] {
        assert!(Snapshot::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn removing_an_active_capability_is_refused_even_on_a_major_version() {
    let new = snap("2.0.0", "1.0.0", "optional", "", "", &[7]);
    let found = lint_transition(&active("1.0.0"), &new);
    assert!(
        found.contains(&LintFinding::RemovedWithoutLifecycle { id: 7, last_status: Status::Active }),
        "{found:?}"
    );
}

#[test]
fn removing_a_deprecated_capability_is_refused_because_it_skipped_write_disabled() {
    let old = snap("1.5.0", "1.0.0", "optional", "deprecated", "1.1.0", &[]);
    let new = snap("2.0.0", "1.0.0", "optional", "", "", &[7]);
    assert!(
        lint_transition(&old, &new)
            .contains(&LintFinding::RemovedWithoutLifecycle { id: 7, last_status: Status::Deprecated })
    );
}

#[test]
fn removal_needs_a_major_version_and_the_retired_list() {
    let old = snap("1.5.0", "1.0.0", "optional", "write-disabled", "1.2.0", &[]);
    let minor = snap("1.6.0", "1.0.0", "optional", "", "", &[7]);
    assert!(
        lint_transition(&old, &minor)
            .contains(&LintFinding::BreakingOnMinor { id: 7, change: Breaking::Removal })
    );
    let no_retire = snap("2.0.0", "1.0.0", "optional", "", "", &[]);
    assert!(lint_transition(&old, &no_retire).contains(&LintFinding::RemovedNotRetired { id: 7 }));
    let ok = snap("2.0.0", "1.0.0", "optional", "", "", &[7]);
    assert_eq!(lint_transition(&old, &ok), vec![]);
}

#[test]
fn the_whole_lifecycle_passes_when_each_state_lasts_long_enough() {
    let a = active("1.0.0");
    let d = snap("1.1.0", "1.0.0", "optional", "deprecated", "1.1.0", &[]);
    let w = snap("1.3.0", "1.0.0", "optional", "write-disabled", "1.3.0", &[]);
    let gone = snap("2.0.0", "1.0.0", "optional", "", "", &[7]);
    assert_eq!(lint_transition(&a, &d), vec![]);
    assert_eq!(lint_transition(&d, &w), vec![]);
    assert_eq!(lint_transition(&w, &gone), vec![]);
}

#[test]
fn leaving_a_state_before_its_minimum_number_of_minor_releases_is_refused() {
    let d = snap("1.1.0", "1.0.0", "optional", "deprecated", "1.1.0", &[]);
    let early = snap("1.2.0", "1.0.0", "optional", "write-disabled", "1.2.0", &[]);
    assert!(lint_transition(&d, &early).contains(&LintFinding::LeftStatusTooEarly {
        id: 7,
        status: Status::Deprecated,
        since: Version::new(1, 1, 0),
        needed_minors: DEPRECATED_MIN_MINORS,
    }));
    let w = snap("1.3.0", "1.0.0", "optional", "write-disabled", "1.3.0", &[]);
    let gone = snap("2.0.0", "1.0.0", "optional", "", "", &[7]);
    // a major release counts as long enough
    assert_eq!(lint_transition(&w, &gone), vec![]);
}

#[test]
fn a_status_may_not_skip_a_step_or_go_back() {
    let a = active("1.0.0");
    let w = snap("1.4.0", "1.0.0", "optional", "write-disabled", "1.4.0", &[]);
    assert!(lint_transition(&a, &w).contains(&LintFinding::StatusSkipped {
        id: 7,
        from: Status::Active,
        to: Status::WriteDisabled
    }));
    let d = snap("1.5.0", "1.0.0", "optional", "deprecated", "1.5.0", &[]);
    assert!(lint_transition(&w, &d).contains(&LintFinding::StatusRegressed {
        id: 7,
        from: Status::WriteDisabled,
        to: Status::Deprecated
    }));
}

#[test]
fn a_breaking_change_on_a_minor_version_is_refused_and_allowed_on_a_major_one() {
    let old = active("1.0.0");
    let bump = snap("1.1.0", "2.0.0", "optional", "active", "1.0.0", &[]);
    assert!(
        lint_transition(&old, &bump)
            .contains(&LintFinding::BreakingOnMinor { id: 7, change: Breaking::MajorBump })
    );
    let tighten = snap("1.1.0", "1.0.0", "required", "active", "1.0.0", &[]);
    assert!(
        lint_transition(&old, &tighten)
            .contains(&LintFinding::BreakingOnMinor { id: 7, change: Breaking::RequirementTightened })
    );
    assert_eq!(lint_transition(&old, &snap("2.0.0", "2.0.0", "required", "active", "1.0.0", &[])), vec![]);
    let minor_add = snap("1.1.0", "1.1.0", "optional", "active", "1.0.0", &[]);
    assert_eq!(lint_transition(&old, &minor_add), vec![], "a minor version of a capability only adds");
}

#[test]
fn a_version_may_not_go_down_and_a_retired_id_may_not_come_back() {
    let old = snap("1.0.0", "1.2.0", "optional", "active", "1.0.0", &[]);
    let down = snap("1.1.0", "1.1.0", "optional", "active", "1.0.0", &[]);
    assert!(lint_transition(&old, &down).contains(&LintFinding::VersionWentDown {
        id: 7,
        old: Version::new(1, 2, 0),
        new: Version::new(1, 1, 0)
    }));
    let back = snap("2.0.0", "1.0.0", "optional", "active", "1.0.0", &[7]);
    assert!(lint_transition(&old, &back).contains(&LintFinding::RetiredIdReused { id: 7 }));
    assert!(lint_transition(&active("1.1.0"), &active("1.0.0")).contains(
        &LintFinding::FormatVersionWentDown { old: Version::new(1, 1, 0), new: Version::new(1, 0, 0) }
    ));
}

#[test]
fn the_step_from_1_0_0_to_1_1_0_that_adds_the_raster_entry_is_a_legal_non_breaking_one() {
    let old = Snapshot::parse(
        &Snapshot::from_registry(&Registry::v1())
            .render()
            .replace("format_version = \"1.3.0\"", "format_version = \"1.0.0\""),
    )
    .unwrap();
    let mut old = old;
    old.entries.retain(|e| e.id < 21);
    let mut new = Snapshot::from_registry(&Registry::v1());
    new.format_version = Version::new(1, 1, 0);
    new.entries.retain(|e| e.id < 22);
    assert_eq!(lint_transition(&old, &new), vec![]);
}

#[test]
fn the_step_from_1_1_0_to_1_2_0_that_adds_the_calibration_entry_is_a_legal_non_breaking_one() {
    let mut new = Snapshot::from_registry(&Registry::v1());
    new.format_version = Version::new(1, 2, 0);
    new.entries.retain(|e| e.id < 23);
    let mut old = new.clone();
    old.format_version = Version::new(1, 1, 0);
    old.entries.retain(|e| e.id != 22);
    assert_eq!(lint_transition(&old, &new), vec![]);
}

#[test]
fn the_step_from_1_2_0_to_1_3_0_that_adds_the_paint_entries_is_a_legal_non_breaking_one() {
    let new = Snapshot::from_registry(&Registry::v1());
    assert_eq!(new.format_version, Version::new(1, 3, 0));
    let mut old = new.clone();
    old.format_version = Version::new(1, 2, 0);
    old.entries.retain(|e| e.id < 23);
    assert_eq!(
        lint_transition(&old, &new),
        vec![],
        "23 and 24 are optional components at 1.0.0 with min reader 1.0.0"
    );
    // the new entries are the two of P1, in id order
    let added: Vec<_> = new.entries.iter().filter(|e| e.id >= 23).map(|e| e.name.as_str()).collect();
    assert_eq!(added, vec!["paint_order", "stroke_paint"]);
}
