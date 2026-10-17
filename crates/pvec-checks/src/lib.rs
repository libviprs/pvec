//! Text checks over the repository itself.
//!
//! These are plain `cargo test` checks so the local MARS gate runs them with everything else. They
//! use only `std`, because a gate that needs a dependency to read its own repository is a gate that
//! can be broken by a registry outage.

use std::fs;
use std::path::{Path, PathBuf};

mod api;
pub use api::*;

/// The one sentence PVec is allowed to say about ISO 10303-42.
pub const APPROVED_CLAIM: &str =
    "PVec is aligned to named ISO 10303-42:2025 constructs and validated through STEP interchange.";

/// Root of the repository, found from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent().and_then(Path::parent).expect("crate lives at <root>/crates/pvec-checks").to_path_buf()
}

/// Every text file the checks care about, relative to `root`, skipping build output and VCS data.
pub fn text_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(name.as_ref(), "target" | ".git" | "node_modules") {
                continue;
            }
            walk(root, &path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "rs" | "toml" | "json" | "txt" | "sh")
        ) {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
}

/// Words that mean "this paragraph is denying or limiting a claim, not making one".
const NEGATIONS: [&str; 9] =
    [" not ", "no ", "never", "nothing", "without", "neither", "cannot", "non-goal", "beyond"];

/// Returns the paragraphs of `text` that assert ISO conformance in wording the policy forbids.
///
/// A paragraph is flagged when it mentions ISO (or 10303) together with conformance wording and
/// carries none of the negation markers. The approved sentence never contains conformance wording,
/// so it needs no special case here.
pub fn forbidden_claims(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for para in text.split("\n\n") {
        let lower = format!(" {} ", para.to_lowercase().replace('\n', " "));
        let mentions_iso = lower.contains("iso") || lower.contains("10303");
        let claims = ["conform", "complian", "certif"].iter().any(|w| lower.contains(w));
        let negated = NEGATIONS.iter().any(|n| lower.contains(n));
        if mentions_iso && claims && !negated {
            hits.push(para.trim().to_string());
        }
    }
    hits
}

/// One capability entry parsed out of `docs/ISO_PROFILE.md`.
#[derive(Debug)]
pub struct Entry {
    /// The section heading, e.g. `1. Cartesian point`.
    pub title: String,
    /// Table rows as (field name, value) pairs, in document order.
    pub fields: Vec<(String, String)>,
    /// The raw text of the section after its heading.
    pub body: String,
}

/// The seven fields every capability table must carry, in order.
pub const REQUIRED_FIELDS: [&str; 7] = [
    "ISO 10303-42:2025 construct",
    "PVec restriction",
    "Allowed dimensions",
    "Parameter-domain convention",
    "Trim convention",
    "Exact Part 21 form",
    "Unsupported Part 42 features",
];

/// Parses the numbered `## n. Title` sections of the profile into entries.
pub fn parse_profile(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    // A `###` subsection (the promotion table) holds tables that are not capability fields.
    let mut fields_open = true;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            fields_open = true;
            let numbered = rest.split('.').next().is_some_and(|n| n.parse::<u32>().is_ok());
            if numbered {
                entries.push(Entry { title: rest.to_string(), fields: Vec::new(), body: String::new() });
            } else {
                // An unnumbered section ends the capability list.
                entries.push(Entry { title: String::new(), fields: Vec::new(), body: String::new() });
            }
            continue;
        }
        if line.starts_with("### ") {
            fields_open = false;
        }
        let Some(cur) = entries.last_mut() else { continue };
        cur.body.push_str(line);
        cur.body.push('\n');
        if !fields_open {
            continue;
        }
        if let Some(row) = line.strip_prefix('|') {
            let cells: Vec<&str> = row.split('|').map(str::trim).collect();
            if cells.len() >= 2 && !cells[0].starts_with("---") && cells[0] != "Field" {
                cur.fields.push((cells[0].to_string(), cells[1].to_string()));
            }
        }
    }
    entries.retain(|e| !e.title.is_empty());
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_a_plain_conformance_claim() {
        assert_eq!(forbidden_claims("PVec fully conforms to ISO 10303-42.").len(), 1);
        assert_eq!(forbidden_claims("The reader is compliant with ISO 10303-42:2025.").len(), 1);
    }

    #[test]
    fn allows_the_approved_sentence_and_negations() {
        assert!(forbidden_claims(APPROVED_CLAIM).is_empty());
        assert!(forbidden_claims("We make no ISO conformance claim.").is_empty());
        assert!(forbidden_claims("PVec never claims ISO 10303-42 conformance.").is_empty());
    }

    #[test]
    fn splits_paragraphs_so_a_negation_elsewhere_does_not_hide_a_claim() {
        let text = "We never overclaim.\n\nPVec is certified against ISO 10303-42.";
        assert_eq!(forbidden_claims(text).len(), 1);
    }
}
