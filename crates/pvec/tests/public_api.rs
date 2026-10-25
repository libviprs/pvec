//! Issue #52: the facade's public API is a checked-in snapshot, and nothing of the container leaks.

use pvec_checks::{banned_wire_names, public_api_snapshot_of_text, rust_files};
use std::fs;
use std::path::Path;

const CORE: [&str; 11] = [
    "PvecReader",
    "PvecWriter",
    "Manifest",
    "CapabilityId",
    "EntityId",
    "Curve",
    "CurveRef",
    "Domain",
    "Bounds2",
    "Transform2",
    "PvecError",
];

const COMPONENTS: [&str; 9] = [
    "LayerStream",
    "LineStyleStream",
    "TextStream",
    "HatchStream",
    "ProvenanceStream",
    "SemanticClassStream",
    "Fill",
    "Stream",
    "Skipped",
];

const UNSTABLE: [&str; 7] =
    ["ViewportQuery", "ReadPlan", "Tier0Plan", "Scene", "Generation", "GenerationClock", "CancelToken"];

/// Never exported: container, codec and source internals.
const NEVER: [&str; 11] = [
    "BlockKind",
    "DirectoryEntry",
    "Header",
    "ContainerWriter",
    "ContainerReader",
    "ContainerError",
    "ReadError",
    "WriteError",
    "pvec_container",
    "pvec_codec",
    "pvec_sources",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The stable snapshot covers every source file but `unstable.rs`, which has its own.
fn snapshots() -> (String, String) {
    let (mut stable, mut unstable) = (String::new(), String::new());
    for f in rust_files(&root().join("src")) {
        let rel = f.strip_prefix(root().join("src")).unwrap().to_string_lossy().replace('\\', "/");
        let snap = public_api_snapshot_of_text(&rel, &fs::read_to_string(&f).unwrap());
        if rel == "unstable.rs" {
            unstable.push_str(&snap);
        } else {
            stable.push_str(&snap);
        }
    }
    (stable, unstable)
}

fn check(file: &str, snap: &str) {
    let path = root().join(file);
    if std::env::var_os("PVEC_BLESS_API").is_some() {
        fs::write(&path, snap).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == snap,
        "the facade API changed. If that is intended, replace {} with:\n----8<----\n{snap}----8<----",
        path.display()
    );
}

#[test]
fn issue_52_the_stable_snapshot_matches_the_committed_file() {
    check("public-api.txt", &snapshots().0);
}

#[test]
fn issue_52_the_unstable_snapshot_matches_the_committed_file() {
    check("public-api-unstable.txt", &snapshots().1);
}

#[test]
fn issue_52_the_snapshot_names_the_frozen_core_and_the_component_payloads() {
    let committed = fs::read_to_string(root().join("public-api.txt")).unwrap_or_default();
    for name in CORE.iter().chain(COMPONENTS.iter()) {
        assert!(committed.contains(name), "public-api.txt does not export {name}");
    }
    for name in UNSTABLE {
        assert!(!committed.contains(name), "{name} is unstable and belongs in public-api-unstable.txt");
    }
}

#[test]
fn issue_52_the_unstable_snapshot_names_the_stream_types() {
    let committed = fs::read_to_string(root().join("public-api-unstable.txt")).unwrap_or_default();
    for name in UNSTABLE {
        assert!(committed.contains(name), "public-api-unstable.txt does not export {name}");
    }
}

#[test]
fn issue_52_no_container_or_serialization_name_is_reachable_through_the_facade() {
    let (stable, unstable) = snapshots();
    for snap in [&stable, &unstable] {
        assert!(banned_wire_names(snap).is_empty(), "wire type in the facade:\n{snap}");
        for name in NEVER {
            assert!(!snap.contains(name), "{name} is reachable through the facade:\n{snap}");
        }
    }
}

#[test]
fn issue_52_the_error_enum_is_non_exhaustive() {
    let committed = fs::read_to_string(root().join("public-api.txt")).unwrap_or_default();
    assert!(
        committed.lines().any(|l| l.contains("#[non_exhaustive]") && l.contains("pub enum PvecError")),
        "PvecError must be #[non_exhaustive]"
    );
}

#[test]
fn issue_52_the_crate_denies_missing_docs() {
    let lib = fs::read_to_string(root().join("src/lib.rs")).unwrap();
    assert!(lib.contains("#![deny(missing_docs)]"));
}

#[test]
fn issue_52_changing_one_parameter_type_turns_the_snapshot_red() {
    let committed = fs::read_to_string(root().join("public-api.txt")).unwrap_or_default();
    let push = committed.lines().find(|l| l.contains("PvecWriter::push(")).expect("PvecWriter::push is in the snapshot");
    let mutated = committed.replace(push, &push.replace("&Curve", "&mut Curve"));
    assert_ne!(mutated, committed, "the mutation must change the snapshot");
}
