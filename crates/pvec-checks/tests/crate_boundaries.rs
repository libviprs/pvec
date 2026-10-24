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
    C {
        /// doc
        x: flatbuffers::Table,
        y: u8,
    },
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
        "Kind::C { x: flatbuffers::Table, y: u8 }",
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

/// The unsafe blocks `crates/pvec-container/src/blocks/aligned.rs` is allowed: the `from_raw_parts` over
/// the `Vec<u64>` storage, the `align_to::<f64>` and the `align_to::<u32>`. Growth fails the check.
const ALIGNED_UNSAFE_BLOCKS: usize = 3;

/// Zero-based line numbers of every `unsafe` token outside `//` comments, matched like `\bunsafe\b`
/// (the crate allow-list has no regex crate, so the boundary is checked by hand): `unsafe_code` and
/// `is_unsafe` are not hits, `unsafe{`, `unsafe fn` and a bare `unsafe` at the end of a line are.
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

#[test]
fn the_unsafe_token_is_matched_on_word_boundaries() {
    assert_eq!(unsafe_tokens("unsafe { x }"), [0]);
    assert_eq!(unsafe_tokens("let a = unsafe{ x };"), [0]);
    assert_eq!(unsafe_tokens("a\nunsafe\n{ x }"), [1]);
    assert_eq!(unsafe_tokens("pub unsafe fn f() {}"), [0]);
    assert_eq!(unsafe_tokens("(unsafe { x })"), [0]);
    assert_eq!(unsafe_tokens("unsafe impl Send for T {}"), [0]);
    assert_eq!(unsafe_tokens("unsafe { a }; unsafe { b }"), [0, 0]);
    assert!(unsafe_tokens("#![allow(unsafe_code)]").is_empty());
    assert!(unsafe_tokens("let is_unsafe = 1; let unsafely = 2;").is_empty());
    assert!(unsafe_tokens("// unsafe { x }").is_empty());
    assert!(unsafe_tokens("let a = 1; // unsafe").is_empty());
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
            // The one module that reinterprets block bytes in place may use `unsafe`, in exactly
            // `ALIGNED_UNSAFE_BLOCKS` places, and every use must carry its argument in a `SAFETY:`
            // comment just above it. The scan covers the whole file (test module included) and matches
            // the token on word boundaries, so `unsafe{`, `unsafe\n` and `unsafe impl` count too.
            let hits = unsafe_tokens(&text);
            if name == &"pvec-container" && file.ends_with("src/blocks/aligned.rs") {
                assert_eq!(
                    hits.len(),
                    ALIGNED_UNSAFE_BLOCKS,
                    "{}: the number of unsafe blocks changed; every new one needs a panel review",
                    file.display()
                );
                let lines: Vec<&str> = text.lines().collect();
                for line_no in hits {
                    assert!(
                        lines[line_no.saturating_sub(5)..line_no].iter().any(|c| c.contains("SAFETY:")),
                        "{}:{}: unsafe without a SAFETY comment",
                        file.display(),
                        line_no + 1
                    );
                }
            } else {
                assert!(hits.is_empty(), "{name}: {} contains unsafe", file.display());
            }
        }
    }
}

#[test]
fn the_public_api_matches_the_committed_snapshot_and_names_no_wire_type() {
    for (name, _) in API_CRATES {
        let snap = public_api_snapshot(&repo_root().join(format!("crates/{name}/src")));
        assert!(banned_wire_names(&snap).is_empty(), "{name}: wire type in the public API:\n{snap}");
        let path = repo_root().join(format!("crates/{name}/api.txt"));
        if std::env::var_os("PVEC_BLESS_API").is_some() {
            fs::write(&path, &snap).unwrap();
        }
        let committed = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == snap,
            "{name}: the public API changed. If that is intended, replace {} with:\n----8<----\n{snap}----8<----",
            path.display()
        );
    }
}

#[test]
fn the_second_evaluator_scan_flags_evaluation_code_outside_the_geometry_crate() {
    assert!(second_evaluator_markers("pub fn eval(&self, t: f64) -> f64 { t }").contains(&"fn eval("));
    assert!(second_evaluator_markers("fn deboor(p: usize) {}").contains(&"deboor"));
    assert!(second_evaluator_markers("fn casteljau(t: f64) {}").contains(&"casteljau"));
    assert!(second_evaluator_markers("let p = curve.eval(0.5);").is_empty(), "calling the evaluator is fine");
    assert!(second_evaluator_markers("fn evaluate_nothing() {}").is_empty());
}

#[test]
fn no_crate_but_the_geometry_crate_contains_an_evaluator() {
    for entry in fs::read_dir(repo_root().join("crates")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "pvec-geometry" || name == "pvec-checks" {
            continue;
        }
        for file in rust_files(&entry.path().join("src")) {
            let code = strip_comments_and_tests(&fs::read_to_string(&file).unwrap());
            let found = second_evaluator_markers(&code);
            assert!(
                found.is_empty(),
                "{name}: {} defines an evaluator ({found:?}); there is exactly one, in pvec-geometry",
                file.display()
            );
        }
    }
}

#[test]
fn the_tessellation_scan_flags_flattening_code() {
    assert!(mentions_tessellation("pub fn tessellate(c: &Curve) -> Vec<Point2> { todo!() }"));
    assert!(mentions_tessellation("let pts = curve.flatten(0.1);"));
    assert!(!mentions_tessellation("let p = curve.eval(0.5);"));
}

#[test]
fn nothing_on_the_geometry_path_calls_a_tessellator() {
    for name in ["pvec-geometry", "pvec-codec", "pvec-entity"] {
        for file in rust_files(&repo_root().join(format!("crates/{name}/src"))) {
            let code = strip_comments_and_tests(&fs::read_to_string(&file).unwrap());
            assert!(
                !mentions_tessellation(&code),
                "{name}: {} mentions tessellation; exact curves are never flattened here",
                file.display()
            );
        }
    }
}

#[test]
fn the_hot_block_module_exposes_no_per_entity_struct() {
    let text = fs::read_to_string(repo_root().join("crates/pvec-container/src/blocks/hot.rs")).unwrap();
    let snap = public_api_snapshot_of_text("blocks/hot.rs", &text);
    let structs: Vec<&str> = snap.lines().filter(|l| l.contains("pub struct ")).collect();
    // The builder and the view are the only structs; neither is one entity read out of the file.
    assert!(structs.iter().any(|l| l.contains("HotBlockView")), "the view is missing:\n{snap}");
    for l in &structs {
        assert!(l.contains("HotBlockView") || l.contains("HotBlockBuilder"), "per-entity struct? {l}");
    }
    for l in snap.lines() {
        for owner in ["HotBlockView.", "HotBlockView<'a>.", "HotBlockBuilder."] {
            assert!(!l.contains(owner), "public field: {l}");
        }
    }
    // every collection the view hands out is a borrowed slice, never an owned Vec
    for l in snap.lines().filter(|l| l.contains("HotBlockView<'a>::")) {
        assert!(!l.contains("Vec<"), "the view returns an owned collection: {l}");
    }
    for name in ["entity_ids", "styles", "run_offsets", "column"] {
        assert!(
            snap.lines().any(|l| l.contains(&format!("HotBlockView<'a>::{name}(")) && l.contains("&'a [")),
            "HotBlockView::{name} does not return a borrowed slice:\n{snap}"
        );
    }
}
