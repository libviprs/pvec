//! Shared helpers: the vendored bundle, and a way to rebuild a modified copy of it that is still
//! internally consistent, so a test can break exactly one thing.

#![allow(dead_code)]

use pvec_corpus::bundle::corpus_dir;
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{FixtureRegistry, Pin, Result, tar};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub type Files = BTreeMap<String, Vec<u8>>;

pub fn repo_root() -> PathBuf {
    corpus_dir().parent().unwrap().parent().unwrap().to_path_buf()
}

pub fn vendored_tar() -> Vec<u8> {
    fs::read(corpus_dir().join("r0-open.tar")).expect("tests/corpus/r0-open.tar is vendored")
}

pub fn vendored_pin() -> Pin {
    Pin::parse(&fs::read(corpus_dir().join("r0-open.pin.json")).expect("pin file is vendored"))
        .expect("pin parses")
}

pub fn profile() -> String {
    fs::read_to_string(repo_root().join("docs/ISO_PROFILE.md")).expect("docs/ISO_PROFILE.md exists")
}

/// The vendored files keyed by their path below the corpus root.
pub fn vendored_files() -> Files {
    let tar = vendored_tar();
    tar::parse(&tar)
        .expect("vendored tar parses")
        .into_iter()
        .map(|e| (e.path.strip_prefix("r0-open/").expect("root is r0-open/").to_string(), e.data.to_vec()))
        .collect()
}

/// One ustar header block, checksum included.
pub fn header(name: &str, size: usize, typeflag: u8) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[148..156].copy_from_slice(b"        ");
    h[156] = typeflag;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// A tar of regular files, given as (full path, bytes).
pub fn tar_of(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        out.extend_from_slice(&header(name, data.len(), b'0'));
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// The deterministic bundle: sorted, rooted at `r0-open/`.
pub fn build_tar(files: &Files) -> Vec<u8> {
    let entries: Vec<(String, Vec<u8>)> =
        files.iter().map(|(k, v)| (format!("r0-open/{k}"), v.clone())).collect();
    tar_of(&entries)
}

/// Regenerates `SHA256SUMS` so it covers the files as they now are.
pub fn reseal(files: &mut Files) {
    files.remove("SHA256SUMS");
    let mut text = String::new();
    for (path, data) in files.iter() {
        text.push_str(&format!("{}  {}\n", hex_digest(data), path));
    }
    files.insert("SHA256SUMS".to_string(), text.into_bytes());
}

/// A pin that matches `files` exactly, taking the rest from the vendored pin.
pub fn pin_for(files: &Files) -> Pin {
    let base = vendored_pin();
    Pin {
        bundle_sha256: hex_digest(&build_tar(files)),
        manifest_sha256: hex_digest(&files["manifest.json"]),
        ..base
    }
}

/// Applies `change`, then reseals, rebuilds the tar and pins it, so the only defect is the one the
/// test introduced.
pub fn mutated(change: impl FnOnce(&mut Files)) -> (Vec<u8>, Pin) {
    let mut files = vendored_files();
    change(&mut files);
    reseal(&mut files);
    let pin = pin_for(&files);
    (build_tar(&files), pin)
}

/// Rewrites one text file of the bundle.
pub fn edit_text(files: &mut Files, path: &str, f: impl FnOnce(String) -> String) {
    let text = String::from_utf8(files[path].clone()).expect("text file");
    files.insert(path.to_string(), f(text).into_bytes());
}

/// Registers a (possibly modified) bundle against the real capability registry and profile.
pub fn register(tar: &[u8], pin: &Pin) -> Result<FixtureRegistry> {
    register_with(tar, pin, &profile())
}

pub fn register_with(tar: &[u8], pin: &Pin, profile: &str) -> Result<FixtureRegistry> {
    let bundle = pvec_corpus::Bundle::open(tar, pin)?;
    FixtureRegistry::load(&bundle, &pvec_capabilities::Registry::v1(), profile)
}

/// Replaces `from` by `to` in the fixture's manifest block, searching backwards from its `"id"` for
/// keys that sort before it (`evidence_class`, `capabilities`) and forwards for the rest.
pub fn edit_manifest_entry(files: &mut Files, id: &str, key: &str, from: &str, to: &str) {
    edit_text(files, "manifest.json", |text| {
        let id_at = text.find(&format!("\"id\": \"{id}\"")).expect("fixture id in manifest");
        let key_at = text[..id_at].rfind(&format!("\"{key}\"")).expect("key before id");
        let from_at = key_at + text[key_at..].find(from).expect("value to replace");
        assert!(from_at < id_at, "replacement stays inside the fixture's block");
        format!("{}{}{}", &text[..from_at], to, &text[from_at + from.len()..])
    });
}
