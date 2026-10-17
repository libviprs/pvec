//! The boundary of the API crates (issues P3.1 to P3.8): no serialization or wire type in a public
//! signature, no file, network or process access, only allow-listed dependencies, and a committed
//! snapshot of the public API that has to be updated on purpose.

use pvec_checks::*;
use std::fs;

#[test]
fn the_scanner_sees_public_items_fields_variants_and_trait_methods() {
    let src = "\
//! docs
pub struct Wire {
    pub inner: serde_json::Value,
    hidden: u8,
}

pub enum Kind {
    A,
    B(u32),
}

pub trait Make {
    fn make(&self, x: flatbuffers::Table) -> u8;
}

impl Wire {
    pub fn load(path: &str) -> Result<Wire,
        String> {
        todo!()
    }
    fn private(&self) {}
    pub(crate) fn crate_only(&self) {}
}

#[cfg(test)]
mod tests {
    pub fn not_api() {}
}
";
    let snap = public_api_snapshot_of_text("lib.rs", src);
    for needle in [
        "pub struct Wire",
        "Wire.inner: serde_json::Value",
        "Kind::A",
        "Kind::B(u32)",
        "Make::make(&self, x: flatbuffers::Table) -> u8",
        "Wire::load(path: &str) -> Result<Wire, String>",
    ] {
        assert!(snap.contains(needle), "missing {needle:?} in\n{snap}");
    }
    for needle in ["hidden", "private", "crate_only", "not_api"] {
        assert!(!snap.contains(needle), "{needle:?} must not be public API:\n{snap}");
    }
}

#[test]
fn the_banned_name_detector_flags_wire_and_serialization_types() {
    assert_eq!(banned_wire_names("pub fn f(x: serde_json::Value)"), vec!["serde"]);
    assert_eq!(banned_wire_names("pub fn f(x: flatbuffers::Table)"), vec!["flatbuffers"]);
    assert_eq!(banned_wire_names("pub fn f(x: ArchivedCurve)"), vec!["Archived"]);
    assert!(banned_wire_names("pub fn f(x: Curve) -> Point2").is_empty());
}

#[test]
fn manifests_list_only_allow_listed_dependencies() {
    for (name, allowed) in API_CRATES {
        let manifest = fs::read_to_string(repo_root().join(format!("crates/{name}/Cargo.toml"))).unwrap();
        let deps = manifest_dependencies(&manifest);
        for d in &deps {
            assert!(allowed.contains(&d.as_str()), "{name} depends on {d}, which is not allow-listed");
        }
        assert!(banned_wire_names(&manifest).is_empty(), "{name}: serialization crate in its manifest");
    }
}

#[test]
fn api_crates_do_no_file_network_or_process_access_and_name_no_wire_crate() {
    let forbidden = ["std::fs", "std::net", "std::process", "std::env", "std::thread", "std::io", "libc::"];
    for (name, _) in API_CRATES {
        let dir = repo_root().join(format!("crates/{name}/src"));
        for file in rust_files(&dir) {
            let text = fs::read_to_string(&file).unwrap();
            let code = strip_comments_and_tests(&text);
            for f in forbidden {
                assert!(!code.contains(f), "{name}: {} uses {f}", file.display());
            }
            assert!(banned_wire_names(&code).is_empty(), "{name}: {} names a wire crate", file.display());
            assert!(!code.contains("unsafe "), "{name}: {} contains unsafe", file.display());
        }
    }
}

#[test]
fn the_public_api_matches_the_committed_snapshot_and_names_no_wire_type() {
    for (name, _) in API_CRATES {
        let snap = public_api_snapshot(&repo_root().join(format!("crates/{name}/src")));
        assert!(banned_wire_names(&snap).is_empty(), "{name}: wire type in the public API:\n{snap}");
        let path = repo_root().join(format!("crates/{name}/api.txt"));
        let committed = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == snap,
            "{name}: the public API changed. If that is intended, replace {} with:\n----8<----\n{snap}----8<----",
            path.display()
        );
    }
}
