//! Issue P8.8 (pvec#67): the complete v1 document set. Every document exists and is reachable from
//! `docs/README.md`, every internal link resolves, only the P1.2 policy defines a tolerance, no document claims
//! formal ISO 10303-42 conformance, and `docs/BENCHMARKS.md` carries every phase 8 verdict.
#![allow(missing_docs)]

use pvec_checks::{forbidden_claims, repo_root};
use std::fs;
use std::path::{Path, PathBuf};

const DOCS: [&str; 20] = [
    "SPECIFICATION.md",
    "ISO_PROFILE.md",
    "NUMERIC_POLICY.md",
    "CONTAINER.md",
    "SECURITY_LIMITS.md",
    "STREAMING.md",
    "FIDELITY_TAXONOMY.md",
    "INTERCHANGE.md",
    "CONFORMANCE.md",
    "API.md",
    "WASM_ABI.md",
    "COMPATIBILITY.md",
    "MIGRATION.md",
    "BENCHMARKS.md",
    "FREEZE-v1.md",
    "CAPABILITIES.md",
    "READER.md",
    "FUZZING.md",
    "GEOMETRY_ENCODING.md",
    "EXTENSIONS.md",
];

fn docs_dir() -> PathBuf {
    repo_root().join("docs")
}

fn read(name: &str) -> String {
    fs::read_to_string(docs_dir().join(name)).unwrap_or_else(|e| panic!("docs/{name}: {e}"))
}

fn markdown_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(docs_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    out.sort();
    out
}

/// `](target)` link targets in a markdown text, without the anchor.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        rest = &rest[i + 2..];
        if let Some(j) = rest.find(')') {
            let t = rest[..j].split('#').next().unwrap_or("").trim().to_string();
            if !t.is_empty() && !t.contains("://") && !t.starts_with("mailto:") && !t.contains(' ') {
                out.push(t);
            }
        }
    }
    out
}

#[test]
fn every_document_of_the_set_exists_and_is_linked_from_the_readme() {
    let readme = read("README.md");
    for name in DOCS {
        assert!(docs_dir().join(name).exists(), "docs/{name} does not exist");
        assert!(links(&readme).iter().any(|l| l == name), "docs/README.md does not link {name}");
    }
}

#[test]
fn every_internal_link_in_the_documents_resolves() {
    let mut broken = Vec::new();
    for file in markdown_files() {
        let text = fs::read_to_string(&file).unwrap();
        for l in links(&text) {
            let target: PathBuf = if let Some(abs) = l.strip_prefix('/') { repo_root().join(abs) } else { file.parent().unwrap().join(&l) };
            if !Path::new(&target).exists() {
                broken.push(format!("{}: {l}", file.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

/// A tolerance-shaped number: an exponent-form literal such as 1e-9, or the word epsilon with a value.
fn tolerance_like(line: &str) -> bool {
    let b: Vec<char> = line.chars().collect();
    let exp = (1..b.len().saturating_sub(2)).any(|i| {
        (b[i] == 'e' || b[i] == 'E')
            && b[i - 1].is_ascii_digit()
            && b[i + 1] == '-'
            && b[i + 2].is_ascii_digit()
    });
    exp || (line.to_lowercase().contains("epsilon") && line.chars().any(|c| c.is_ascii_digit()))
}

#[test]
fn only_the_numeric_policy_defines_a_tolerance_and_every_other_mention_cites_it() {
    let mut loose = Vec::new();
    for file in markdown_files() {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        if name == "NUMERIC_POLICY.md" || file.parent().is_some_and(|p| p.ends_with("spikes")) {
            continue;
        }
        for (n, line) in fs::read_to_string(&file).unwrap().lines().enumerate() {
            if tolerance_like(line) && !(line.contains("NUMERIC_POLICY") || line.contains("NP-") || line.contains("P1.2")) {
                loose.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(loose.is_empty(), "a tolerance outside the P1.2 policy, with no citation of it:\n{}", loose.join("\n"));
}

#[test]
fn no_document_claims_formal_or_full_iso_conformance() {
    for name in DOCS {
        let hits = forbidden_claims(&read(name));
        assert!(hits.is_empty(), "docs/{name} claims conformance: {hits:#?}");
    }
}

#[test]
fn the_conformance_page_keeps_three_verdicts_apart_and_says_display_perfect_is_not_exact() {
    let c = read("CONFORMANCE.md");
    for verdict in ["structural exactness", "geometric exactness", "display fidelity"] {
        assert!(c.contains(verdict), "CONFORMANCE.md never says {verdict:?}");
    }
    assert!(c.contains("a display-perfect result is not mathematically exact"));
}

#[test]
fn the_specification_has_a_third_party_reader_checklist_that_points_at_the_worked_example() {
    let s = read("SPECIFICATION.md");
    assert!(s.contains("## Third-party reader checklist"));
    assert!(s.contains("CONTAINER.md#6-an-annotated-example") || s.contains("an annotated example"));
    for step in ["enumerate the required capabilities", "parse the header", "skip an unknown optional block"] {
        assert!(s.contains(step), "the checklist never says to {step}");
    }
    assert!(s.contains("FREEZE-v1.md"), "the specification is assembled from the frozen rows");
}

/// The verdict lines `- id on platform: PASS (...)` of a benchmark report.
fn verdict_ids(report: &str) -> Vec<(String, String, String)> {
    let text = fs::read_to_string(docs_dir().join("bench").join(report)).unwrap();
    let mut out: Vec<(String, String, String)> = Vec::new();
    for l in text.lines().filter_map(|l| l.strip_prefix("- ")) {
        let Some((id, rest)) = l.split_once(" on ") else { continue };
        let Some((platform, verdict)) = rest.split_once(": ") else { continue };
        let word = verdict.split_whitespace().next().unwrap_or("");
        if ["PASS", "FAIL"].contains(&word) && platform.starts_with("linux-") {
            let row = (id.to_string(), platform.to_string(), word.to_string());
            if !out.contains(&row) {
                out.push(row);
            }
        }
    }
    out
}

#[test]
fn the_benchmarks_page_reproduces_every_phase_8_threshold_with_its_platform_and_the_corpus_digest() {
    let b = read("BENCHMARKS.md");
    let lock = fs::read_to_string(repo_root().join("crates/pvec-bench/benches/corpus/corpus.lock.json")).unwrap();
    let digest = pvec_checks_digest(&lock);
    assert!(b.contains(&digest), "BENCHMARKS.md does not name the corpus digest {digest}");
    let mut seen = 0;
    for report in ["STEP_COMPARISON.md", "LAYOUT.md", "GEOMETRY.md"] {
        for (id, platform, verdict) in verdict_ids(report) {
            seen += 1;
            let row = b.lines().find(|l| l.starts_with('|') && l.contains(&format!("`{id}`")) && l.contains(&platform));
            let row = row.unwrap_or_else(|| panic!("BENCHMARKS.md has no row for {id} on {platform}"));
            assert!(row.contains(&verdict), "{id} on {platform}: the row does not say {verdict}");
        }
    }
    assert!(seen >= 30, "found only {seen} phase 8 verdicts to reproduce");
    for missing in ["PROGRESSIVE.md", "TUNING.md"] {
        assert!(b.contains(missing) && b.contains("not measured"), "BENCHMARKS.md hides that {missing} is missing");
    }
}

/// sha256 of the lock file, through the one implementation the repository tests share.
fn pvec_checks_digest(text: &str) -> String {
    pvec_corpus::sha256::hex_digest(text.as_bytes())
}
