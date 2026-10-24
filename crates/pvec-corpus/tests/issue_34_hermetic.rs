//! Issue #34: the suite is hermetic, and that is asserted here rather than assumed. The bundle is a
//! vendored tar, so the loader never needs a container: nothing can be skipped for want of Docker,
//! and nothing can reach a converter.

mod common;

use common::*;
use pvec_corpus::json::Value;
use pvec_corpus::{Bundle, bundle::corpus_dir};
use std::fs;

fn sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.display().to_string(), fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn issue_34_the_loader_starts_no_process_and_opens_no_socket() {
    let crate_dir = repo_root().join("crates/pvec-corpus");
    let mut files = Vec::new();
    sources(&crate_dir.join("src"), &mut files);
    assert!(files.len() >= 10, "found {} source files", files.len());
    // Built from pieces so this file does not trip its own scan.
    let banned = [
        ["std::", "process"].concat(),
        ["std::", "net"].concat(),
        ["Command", "::new"].concat(),
        ["Tcp", "Stream"].concat(),
        ["Udp", "Socket"].concat(),
        ["doc", "ker"].concat(),
        ["cu", "rl "].concat(),
        ["http", "://"].concat(),
        ["https", "://"].concat(),
    ];
    for (name, text) in &files {
        let code: String =
            text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for b in &banned {
            assert!(!code.contains(b.as_str()), "{name} contains `{b}`");
        }
    }
}

#[test]
fn issue_34_the_crate_depends_on_nothing_outside_the_workspace() {
    let manifest = fs::read_to_string(repo_root().join("crates/pvec-corpus/Cargo.toml")).unwrap();
    let deps: Vec<&str> = manifest
        .lines()
        .skip_while(|l| l.trim() != "[dependencies]")
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .filter(|l| l.contains('='))
        .collect();
    assert_eq!(deps.len(), 2, "{deps:?}");
    assert!(deps.iter().all(|d| d.contains("path = \"../")), "{deps:?}");
}

#[test]
fn issue_34_the_vendored_directory_holds_the_tar_and_its_pin_and_nothing_else() {
    let mut names: Vec<String> = fs::read_dir(corpus_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["r0-open.pin.json", "r0-open.tar"]);
}

#[test]
fn issue_34_no_cad_exchanger_or_oda_image_or_output_is_part_of_the_bundle() {
    let bundle = Bundle::vendored().unwrap();
    let m = bundle.manifest();
    let text = |key: &str| {
        m.get(key).and_then(Value::as_str).unwrap_or_else(|| panic!("manifest has {key}")).to_string()
    };
    assert_eq!(text("converters"), "none");
    assert_eq!(text("oracles"), "none");
    assert_eq!(text("step_oracle"), "none");
    assert!(text("evaluator").starts_with("none"));
    assert_eq!(text("evidence_scope"), "READER_MAPPING_ONLY");
    // The only images the corpus names are the Rust writer image and the .NET reader image.
    let images = m.get("container_images").and_then(Value::as_obj).unwrap();
    for (role, image) in images {
        let image = image.as_str().unwrap().to_lowercase();
        for vendor in ["cadex", "cad exchanger", "oda", "teigha", "opendesign"] {
            assert!(!image.contains(vendor), "{role}: {image}");
        }
    }
    // No STEP output and no vendor-converter directory ships with it.
    for p in bundle.paths() {
        let lower = p.to_lowercase();
        assert!(!lower.ends_with(".step") && !lower.ends_with(".stp") && !lower.ends_with(".p21"), "{p}");
        assert!(!lower.contains("cadex") && !lower.contains("/oda"), "{p}");
    }
    // Every fixture says so too.
    for f in m.get("fixtures").and_then(Value::as_arr).unwrap() {
        assert_eq!(f.get("oracles").and_then(Value::as_str), Some("none"));
        assert_eq!(f.get("evidence_scope").and_then(Value::as_str), Some("READER_MAPPING_ONLY"));
    }
}
