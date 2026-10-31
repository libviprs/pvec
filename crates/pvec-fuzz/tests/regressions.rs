//! Issue P7.8: every crash is fixed by a bounds or limit check and keeps a reproducer in
//! `tests/regressions/<target>/`; each one replays clean.

mod support;

use pvec_fuzz::TARGETS;
use support::*;

#[test]
fn every_checked_in_reproducer_replays_without_a_panic() {
    let root = repo().join("tests/regressions");
    assert!(root.is_dir(), "tests/regressions exists");
    let mut replayed = 0;
    for (name, target) in TARGETS {
        let dir = root.join(name);
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') || entry.file_name() == "README.md" {
                continue;
            }
            let bytes = std::fs::read(entry.path()).unwrap();
            assert_eq!(panics(|| drop(target(&bytes))), None, "{}", entry.path().display());
            replayed += 1;
        }
    }
    println!("replayed {replayed} reproducers");
}

#[test]
fn the_readme_says_how_a_reproducer_is_added() {
    let text = std::fs::read_to_string(repo().join("tests/regressions/README.md")).unwrap();
    assert!(text.contains("fuzz_") && text.contains("bounds") && text.contains("limit"));
}

#[test]
fn the_readme_names_the_directory_of_every_target_including_the_raster_index() {
    let text = std::fs::read_to_string(repo().join("tests/regressions/README.md")).unwrap();
    for (name, _) in TARGETS {
        assert!(text.contains(name), "tests/regressions/README.md does not name {name}");
    }
    assert!(TARGETS.iter().any(|(n, _)| *n == "fuzz_raster_index"));
}
