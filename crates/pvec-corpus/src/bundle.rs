//! The bundle: the vendored tar, checked against the pin before anything in it is trusted.

use crate::error::{CorpusError, Result};
use crate::json::{self, Value};
use crate::pin::Pin;
use crate::sha256::hex_digest;
use crate::tar;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The numeric policy this loader compares under.
pub const SUPPORTED_POLICY: &str = "PVec numeric policy v1";

const ROOT: &str = "r0-open/";
const MANIFEST_SCHEMA: &str = "pvec-gold/corpus-manifest/1";

/// A verified bundle held in memory.
#[derive(Clone, Debug)]
pub struct Bundle {
    pin: Pin,
    files: BTreeMap<String, Vec<u8>>,
    manifest: Value,
}

impl Bundle {
    /// Opens a tar against a pin. Refuses a bundle whose recorded revision differs (naming both),
    /// whose digest differs, whose manifest digest differs, or any of whose files disagrees with its
    /// `SHA256SUMS`.
    pub fn open(tar_bytes: &[u8], pin: &Pin) -> Result<Bundle> {
        let mut files = BTreeMap::new();
        for entry in tar::parse(tar_bytes)? {
            let Some(path) = entry.path.strip_prefix(ROOT) else {
                return Err(CorpusError::Tar(format!("`{}` is outside the root `{ROOT}`", entry.path)));
            };
            files.insert(path.to_string(), entry.data.to_vec());
        }
        let manifest_bytes = files
            .get("manifest.json")
            .ok_or_else(|| CorpusError::Missing("manifest.json in the bundle".to_string()))?;
        let manifest = json::parse(manifest_bytes)?;

        // The revision is read before the digest, so a bundle of another revision says which one it is
        // instead of only saying that its hash is wrong.
        let recorded = manifest
            .get("corpus_revision")
            .and_then(Value::as_str)
            .ok_or_else(|| CorpusError::Missing("corpus_revision in the manifest".to_string()))?;
        if recorded != pin.corpus_revision {
            return Err(CorpusError::RevisionMismatch {
                pinned: pin.corpus_revision.clone(),
                recorded: recorded.to_string(),
            });
        }
        let actual = hex_digest(tar_bytes);
        if actual != pin.bundle_sha256 {
            return Err(CorpusError::HashMismatch {
                what: "the bundle".to_string(),
                pinned: pin.bundle_sha256.clone(),
                actual,
            });
        }
        let actual = hex_digest(manifest_bytes);
        if actual != pin.manifest_sha256 {
            return Err(CorpusError::HashMismatch {
                what: "manifest.json".to_string(),
                pinned: pin.manifest_sha256.clone(),
                actual,
            });
        }
        check_sums(&files)?;

        if manifest.get("schema_version").and_then(Value::as_str) != Some(MANIFEST_SCHEMA) {
            return Err(CorpusError::Invalid {
                what: "manifest schema_version".to_string(),
                why: format!("only `{MANIFEST_SCHEMA}` is understood"),
            });
        }
        let manifest_policy = manifest.get("numeric_policy_version").and_then(Value::as_str);
        if manifest_policy != Some(pin.numeric_policy_version.as_str())
            || pin.numeric_policy_version != SUPPORTED_POLICY
        {
            return Err(CorpusError::Invalid {
                what: "numeric policy version".to_string(),
                why: format!(
                    "the pin says `{}`, the manifest says `{}`, and this loader compares under `{SUPPORTED_POLICY}`",
                    pin.numeric_policy_version,
                    manifest_policy.unwrap_or("nothing")
                ),
            });
        }
        Ok(Bundle { pin: pin.clone(), files, manifest })
    }

    /// The bundle vendored under `tests/corpus/`, opened against the pin next to it. Reads the two
    /// files from the repository checkout; nothing is fetched.
    pub fn vendored() -> Result<Bundle> {
        let dir = corpus_dir();
        let read = |name: &str| {
            std::fs::read(dir.join(name))
                .map_err(|e| CorpusError::Missing(format!("{} ({e})", dir.join(name).display())))
        };
        let pin = Pin::parse(&read("r0-open.pin.json")?)?;
        Bundle::open(&read("r0-open.tar")?, &pin)
    }

    /// The pin it was opened against.
    pub fn pin(&self) -> &Pin {
        &self.pin
    }
    /// The parsed `manifest.json`.
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    /// A file by its path below the corpus root, for example `fixtures/circle-001/source.dwg`.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
    /// Every path in the bundle, sorted.
    pub fn paths(&self) -> Vec<&str> {
        self.files.keys().map(String::as_str).collect()
    }
}

/// `SHA256SUMS` lists every file but itself, as `<digest>  <path>`.
fn check_sums(files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let sums = files
        .get("SHA256SUMS")
        .ok_or_else(|| CorpusError::Missing("SHA256SUMS in the bundle".to_string()))?;
    let text = std::str::from_utf8(sums)
        .map_err(|_| CorpusError::Invalid { what: "SHA256SUMS".to_string(), why: "not UTF-8".to_string() })?;
    let mut listed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for line in text.lines() {
        let (digest, path) = line.split_once("  ").ok_or_else(|| CorpusError::Invalid {
            what: "SHA256SUMS".to_string(),
            why: format!("line `{line}` is not `<digest>  <path>`"),
        })?;
        if !listed.insert(path) {
            return Err(CorpusError::Invalid {
                what: "SHA256SUMS".to_string(),
                why: format!("`{path}` is listed twice"),
            });
        }
        let data =
            files.get(path).ok_or_else(|| CorpusError::Missing(format!("{path}, which SHA256SUMS lists")))?;
        let actual = hex_digest(data);
        if actual != digest {
            return Err(CorpusError::HashMismatch {
                what: path.to_string(),
                pinned: digest.to_string(),
                actual,
            });
        }
    }
    // The listed set has to be exactly the files present, SHA256SUMS itself excepted.
    if let Some(unlisted) =
        files.keys().map(String::as_str).find(|p| *p != "SHA256SUMS" && !listed.contains(p))
    {
        return Err(CorpusError::Invalid {
            what: "SHA256SUMS".to_string(),
            why: format!("`{unlisted}` is in the bundle but not listed"),
        });
    }
    Ok(())
}

/// The repository's `tests/corpus` directory.
pub fn corpus_dir() -> PathBuf {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent().and_then(std::path::Path::parent).unwrap_or(manifest_dir);
    root.join("tests").join("corpus")
}
