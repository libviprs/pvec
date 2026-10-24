//! Issue #108: where `unsafe` is allowed, now that a memory map needs one block of it.
//!
//! The rule is a short list, not a pattern: `pvec-container`'s `blocks/aligned.rs` with its three
//! casts, and `pvec-mmap`'s `src/map.rs` with the one call that maps a file (ADR 0016). Every other
//! source file of every crate under `crates/` has none, and every block carries its argument in a
//! `SAFETY:` comment just above it. A new site, or a new dependency of the mmap crate, fails here until
//! the ADR and this list change on purpose.

use pvec_checks::*;
use std::fs;

const ALIGNED: (&str, usize) = ("crates/pvec-container/src/blocks/aligned.rs", 3);
const MAP: (&str, usize) = ("crates/pvec-mmap/src/map.rs", 1);

/// Zero-based line numbers of every `unsafe` token outside `//` comments, on word boundaries. The same
/// matcher as `crate_boundaries.rs` (and not shared with it: a shared one would live in the library
/// source, where its own string literal would be a hit for this scan).
fn unsafe_tokens(text: &str) -> Vec<usize> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut hits = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        for (at, _) in code.match_indices("unsafe") {
            let before = code[..at].chars().next_back();
            let after = code[at + 6..].chars().next();
            if !before.is_some_and(word) && !after.is_some_and(word) {
                hits.push(n);
            }
        }
    }
    hits
}

fn crate_sources() -> Vec<std::path::PathBuf> {
    let mut all = Vec::new();
    for entry in fs::read_dir(repo_root().join("crates")).unwrap() {
        let dir = entry.unwrap().path().join("src");
        if dir.is_dir() {
            all.extend(rust_files(&dir));
        }
    }
    all.sort();
    all
}

#[test]
fn unsafe_appears_in_exactly_the_two_approved_files_and_nowhere_else_in_any_crate_source() {
    let sources = crate_sources();
    assert!(sources.len() > 20, "the scan found only {} files", sources.len());
    let mut approved_seen = 0;
    for file in sources {
        let rel = file.strip_prefix(repo_root()).unwrap().to_string_lossy().replace('\\', "/");
        let text = fs::read_to_string(&file).unwrap();
        let hits = unsafe_tokens(&text);
        match [ALIGNED, MAP].iter().find(|(path, _)| *path == rel) {
            Some((_, count)) => {
                approved_seen += 1;
                assert_eq!(hits.len(), *count, "{rel}: the number of unsafe blocks changed; ADR 0016");
                let lines: Vec<&str> = text.lines().collect();
                for at in hits {
                    assert!(
                        lines[at.saturating_sub(5)..at].iter().any(|l| l.contains("SAFETY:")),
                        "{rel}:{}: unsafe without a SAFETY comment just above it",
                        at + 1
                    );
                }
            }
            None => assert!(hits.is_empty(), "{rel} contains unsafe, which only the approved files may"),
        }
    }
    assert_eq!(approved_seen, 2, "both approved files exist");
}

#[test]
fn unsafe_code_is_denied_in_the_mmap_crate_except_in_its_one_module() {
    let root = repo_root().join("crates/pvec-mmap/src");
    for file in rust_files(&root) {
        let text = fs::read_to_string(&file).unwrap();
        let allows = text.contains("allow(unsafe_code)");
        if file.ends_with("map.rs") {
            assert!(allows, "map.rs has to allow unsafe_code, on the module and nowhere wider");
        } else {
            assert!(!allows, "{} allows unsafe_code", file.display());
        }
    }
    let lib = fs::read_to_string(root.join("lib.rs")).unwrap();
    assert!(lib.contains("#![deny(unsafe_code)]"), "lib.rs does not deny unsafe_code");
}

#[test]
fn the_mmap_crate_depends_on_the_container_crate_and_one_vetted_map_crate_and_no_api_crate_on_it() {
    let manifest = fs::read_to_string(repo_root().join("crates/pvec-mmap/Cargo.toml")).unwrap();
    let mut deps = manifest_dependencies(&manifest);
    deps.sort();
    assert_eq!(deps, ["memmap2", "pvec-container"]);
    let line = manifest.lines().find(|l| l.starts_with("memmap2")).unwrap();
    assert!(
        line.contains('=') && !line.contains("git") && !line.contains("path"),
        "{line}: a registry version"
    );
    for (name, allowed) in API_CRATES {
        assert!(!allowed.contains(&"memmap2") && !allowed.contains(&"pvec-mmap"), "{name} allows the map");
        let m = fs::read_to_string(repo_root().join(format!("crates/{name}/Cargo.toml"))).unwrap();
        assert!(!m.contains("memmap2") && !m.contains("pvec-mmap"), "{name} depends on the map");
    }
}

#[test]
fn adr_0016_records_the_decision() {
    let t = fs::read_to_string(repo_root().join("docs/adr/0016-memory-mapped-source.md")).expect("ADR 0016");
    assert!(t.starts_with("# ADR 0016"));
    for h in ["Context", "Options", "Decision", "Consequences", "Reversal threshold"] {
        assert!(t.contains(&format!("\n## {h}")), "no '{h}' section");
    }
    for needle in ["memmap2", "SIGBUS", "map.rs", "aligned.rs", "pvec-mmap"] {
        assert!(t.contains(needle), "ADR 0016 does not mention {needle}");
    }
}
