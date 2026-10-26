//! Issue #35: the licensing gate. This repository consumes converter artifacts and never carries a
//! proprietary converter image or a credential. The scan is asserted, not assumed, and it is shown to
//! see what it is looking for.

use std::fs;
use std::path::{Path, PathBuf};

/// Names of the converter vendors, spelled in two halves so this file does not match its own scan.
fn vendor_words() -> Vec<String> {
    vec![
        ["cad", "ex"].concat(),
        ["cad", " exchanger"].concat(),
        ["cad", "-exchanger"].concat(),
        "oda".to_string(),
        ["open", "design"].concat(),
    ]
}

fn secret_patterns() -> Vec<String> {
    vec![
        ["-----BEGIN ", "PRIVATE KEY"].concat(),
        ["-----BEGIN ", "RSA PRIVATE KEY"].concat(),
        ["gh", "p_"].concat(),
        ["AK", "IA"].concat(),
        ["LICENSE", "_KEY="].concat(),
    ]
}

/// What `tree` contains that this repository must not: a container build or CI file that names a
/// converter vendor, or a credential pattern anywhere.
fn violations(tree: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![tree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path: PathBuf = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | ".git" | "node_modules") {
                    stack.push(path);
                }
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            if bytes.len() > 4 << 20 {
                continue;
            }
            let Ok(text) = String::from_utf8(bytes) else { continue };
            let lower = text.to_lowercase();
            let is_build_file = name.starts_with("Dockerfile")
                || name.starts_with("docker-compose")
                || name.ends_with(".yml")
                || name.ends_with(".yaml");
            if is_build_file {
                for word in vendor_words() {
                    let named_as_image = lower.lines().any(|l| {
                        let l = l.trim();
                        (l.starts_with("from ") || l.starts_with("image:") || l.starts_with("- image:"))
                            && l.contains(&word)
                    });
                    if named_as_image {
                        found.push(format!("{}: pulls an image that names `{word}`", path.display()));
                    }
                }
            }
            for p in secret_patterns() {
                if text.contains(&p) {
                    found.push(format!("{}: contains the credential pattern `{p}`", path.display()));
                }
            }
        }
    }
    found
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn issue_35_the_repository_carries_no_converter_image_and_no_credential() {
    let v = violations(&repo_root());
    assert!(v.is_empty(), "licensing gate:\n{}", v.join("\n"));
}

#[test]
fn issue_35_the_scan_flags_a_converter_image_and_a_credential_when_they_are_present() {
    let dir = repo_root().join("target/issue-35-scan");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".github/workflows")).unwrap();
    let image = ["registry.example/cad", "ex/sdk:3"].concat();
    fs::write(dir.join("Dockerfile"), format!("FROM {image}\n")).unwrap();
    fs::write(
        dir.join(".github/workflows/ci.yml"),
        format!("jobs:\n  a:\n    container:\n      image: {image}\n"),
    )
    .unwrap();
    fs::write(dir.join("notes.txt"), format!("{}abc", ["gh", "p_"].concat())).unwrap();
    let v = violations(&dir);
    assert!(v.iter().any(|m| m.contains("Dockerfile")), "{v:?}");
    assert!(v.iter().any(|m| m.contains("ci.yml")), "{v:?}");
    assert!(v.iter().any(|m| m.contains("notes.txt")), "{v:?}");
    fs::remove_dir_all(&dir).unwrap();
}
