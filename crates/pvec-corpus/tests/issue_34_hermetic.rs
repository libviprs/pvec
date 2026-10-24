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

/// Tokens of Rust source with comments and string or char literals removed. Identifiers, `::`, braces,
/// commas, semicolons, `*` and the rest as single characters.
fn tokens(src: &str) -> Vec<String> {
    let b: Vec<char> = src.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        let c = b[i];
        if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if c == '\'' {
            // A char literal (`'x'`, `'\\n'`) or a lifetime (`'a`).
            if b.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < b.len() && b[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if b.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if c == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                i += if b[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            out.push(b[start..i].iter().collect());
        } else if c == ':' && b.get(i + 1) == Some(&':') {
            out.push("::".to_string());
            i += 2;
        } else {
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
            i += 1;
        }
    }
    out
}

/// Expands one use tree starting at `t[*i]` into full paths.
fn use_tree(t: &[String], i: &mut usize, prefix: &[String], out: &mut Vec<Vec<String>>) {
    if t.get(*i).map(String::as_str) == Some("{") {
        *i += 1;
        while *i < t.len() && t[*i] != "}" {
            use_tree(t, i, prefix, out);
            if t.get(*i).map(String::as_str) == Some(",") {
                *i += 1;
            }
        }
        *i += 1;
        return;
    }
    let mut path = prefix.to_vec();
    while *i < t.len() {
        match t[*i].as_str() {
            "::" => *i += 1,
            "{" => {
                use_tree(t, i, &path, out);
                return;
            }
            "," | "}" | ";" => break,
            "as" => {
                *i += 2;
                break;
            }
            "*" | "self" => *i += 1,
            seg => {
                path.push(seg.to_string());
                *i += 1;
            }
        }
    }
    out.push(path);
}

/// Every path a source file imports with `use`, plus every `std::...` path written inline.
fn paths(src: &str) -> Vec<String> {
    let t = tokens(src);
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i] == "use" {
            i += 1;
            use_tree(&t, &mut i, &[], &mut out);
        } else if t[i] == "std" || t[i] == "core" {
            let mut path = vec![t[i].clone()];
            i += 1;
            while t.get(i).map(String::as_str) == Some("::") && i + 1 < t.len() && t[i + 1] != "{" {
                path.push(t[i + 1].clone());
                i += 2;
            }
            out.push(path);
        } else {
            i += 1;
        }
    }
    out.into_iter().map(|p| p.join("::")).collect()
}

/// The imports a hermetic loader must not have.
fn banned_paths(src: &str) -> Vec<String> {
    const BANNED: [&str; 5] = ["std::process", "std::net", "std::os", "core::net", "std::env"];
    paths(src)
        .into_iter()
        .filter(|p| BANNED.iter().any(|b| p == b || p.starts_with(&format!("{b}::"))))
        .collect()
}

#[test]
fn issue_34_the_scanner_sees_grouped_nested_aliased_and_inline_paths() {
    assert_eq!(banned_paths("use std::{net, process};"), ["std::net", "std::process"]);
    assert_eq!(
        banned_paths("use std::{io, process::{self, Command}};"),
        ["std::process", "std::process::Command"]
    );
    assert_eq!(banned_paths("use std::net as n;"), ["std::net"]);
    assert_eq!(banned_paths("use std::net::*;"), ["std::net"]);
    assert_eq!(banned_paths("use ::std::process::Command;"), ["std::process::Command"]);
    assert!(
        banned_paths("let q = b'\"'; let a: &'static str = \"\";").is_empty(),
        "quote chars do not open a string"
    );
    assert_eq!(
        banned_paths("fn f() { let _ = std::net::TcpStream::connect(a); }"),
        ["std::net::TcpStream::connect"]
    );
    assert_eq!(banned_paths("use std::{fs, env::var};"), ["std::env::var"]);
    assert_eq!(
        banned_paths("use std::os::unix::process::CommandExt;"),
        ["std::os::unix::process::CommandExt"]
    );
    assert!(banned_paths("use std::{fs, io::{self, Read}, path::PathBuf};").is_empty());
    assert!(banned_paths("// use std::net;\n/* use std::process; */ let s = \"use std::net;\";").is_empty());
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
        let found = banned_paths(text);
        assert!(found.is_empty(), "{name} imports {found:?}");
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
