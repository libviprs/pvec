//! Issue #34: the corpus revision is pinned in the repository, and a bundle that is not the pinned
//! one is refused before anything in it is used.

mod common;

use common::*;
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{Bundle, CorpusError, Pin};

#[test]
fn issue_34_the_vendored_pin_names_the_revision_tag_and_digest() {
    let pin = vendored_pin();
    assert_eq!(pin.corpus_revision, "r0-open");
    assert_eq!(pin.tag, "corpus-r0-open");
    assert_eq!(pin.bundle_sha256, "901370036bbffbbb8cb30471c2bf90f171b4f966c5bdfea6b140591cd840ca50");
    assert_eq!(pin.manifest_sha256, "de3903df3e4d7bc0556a2d593298aaea34dcdbdcdec9015d753df14f1566bf21");
    assert_eq!(pin.numeric_policy_version, "PVec numeric policy v1");
}

#[test]
fn issue_34_the_vendored_bundle_opens_against_its_pin_with_nothing_fetched() {
    let bundle = Bundle::vendored().expect("vendored bundle opens");
    assert_eq!(bundle.pin(), &vendored_pin());
    assert_eq!(bundle.manifest().get("corpus_revision").and_then(|v| v.as_str()), Some("r0-open"));
    assert!(bundle.file("manifest.json").is_some());
    assert!(bundle.file("fixtures/circle-001/source.dwg").is_some());
    assert!(bundle.file("../pin").is_none());
}

#[test]
fn issue_34_a_bundle_recording_another_revision_is_refused_naming_both() {
    let (tar, pin) = mutated(|f| {
        edit_text(f, "manifest.json", |t| {
            t.replacen("\"corpus_revision\": \"r0-open\"", "\"corpus_revision\": \"r1-next\"", 1)
        });
    });
    assert_eq!(pin.corpus_revision, "r0-open", "the repository still pins r0-open");
    match Bundle::open(&tar, &pin) {
        Err(e @ CorpusError::RevisionMismatch { .. }) => {
            let CorpusError::RevisionMismatch { pinned, recorded } = &e else { unreachable!() };
            assert_eq!((pinned.as_str(), recorded.as_str()), ("r0-open", "r1-next"));
            let msg = e.to_string();
            assert!(msg.contains("r0-open") && msg.contains("r1-next"), "{msg}");
        }
        other => panic!("expected a revision mismatch, got {other:?}"),
    }
}

#[test]
fn issue_34_a_bundle_with_the_wrong_digest_is_refused_naming_both_digests() {
    let mut tar = vendored_tar();
    // Flip a bit inside a DWG's bytes, so the tar still parses and only the digest can notice.
    let at = {
        let entries = pvec_corpus::tar::parse(&tar).unwrap();
        let dwg = entries.iter().find(|e| e.path == "r0-open/fixtures/circle-001/source.dwg").unwrap();
        dwg.data.as_ptr() as usize - tar.as_ptr() as usize + 3
    };
    tar[at] ^= 0x01;
    let pin = vendored_pin();
    match Bundle::open(&tar, &pin) {
        Err(CorpusError::HashMismatch { pinned, actual, .. }) => {
            assert_eq!(pinned, pin.bundle_sha256);
            assert_eq!(actual, hex_digest(&tar));
            assert_ne!(pinned, actual);
        }
        other => panic!("expected a digest mismatch, got {other:?}"),
    }
}

#[test]
fn issue_34_a_file_that_disagrees_with_sha256sums_is_refused_by_name() {
    let (tar, pin) = {
        let mut files = vendored_files();
        files.get_mut("fixtures/circle-001/expected.math.json").unwrap().push(b' ');
        // Deliberately not resealed: the pin matches the tar, SHA256SUMS does not match the file.
        let pin = pin_for(&files);
        (build_tar(&files), pin)
    };
    match Bundle::open(&tar, &pin) {
        Err(CorpusError::HashMismatch { what, .. }) => {
            assert!(what.contains("fixtures/circle-001/expected.math.json"), "{what}")
        }
        other => panic!("expected a per-file digest mismatch, got {other:?}"),
    }
}

#[test]
fn issue_34_a_manifest_that_is_not_the_pinned_one_is_refused() {
    let (tar, pin) = mutated(|f| edit_text(f, "manifest.json", |t| format!("{t} ")));
    let pin = Pin { manifest_sha256: vendored_pin().manifest_sha256, ..pin };
    match Bundle::open(&tar, &pin) {
        Err(CorpusError::HashMismatch { what, pinned, .. }) => {
            assert!(what.contains("manifest.json"), "{what}");
            assert_eq!(pinned, vendored_pin().manifest_sha256);
        }
        other => panic!("expected a manifest digest mismatch, got {other:?}"),
    }
}

#[test]
fn issue_34_a_bundle_authored_against_another_numeric_policy_is_refused() {
    let (tar, pin) = mutated(|f| {
        edit_text(f, "manifest.json", |t| {
            t.replacen(
                "\"numeric_policy_version\": \"PVec numeric policy v1\"",
                "\"numeric_policy_version\": \"PVec numeric policy v2\"",
                1,
            )
        });
    });
    match Bundle::open(&tar, &pin) {
        Err(e) => assert!(e.to_string().contains("numeric policy"), "{e}"),
        Ok(_) => panic!("a different numeric policy must be refused, not compared under v1"),
    }
}

#[test]
fn issue_34_a_malformed_pin_file_is_refused() {
    for text in [
        "{}",
        r#"{"corpus_revision": "r0-open"}"#,
        r#"{"corpus_revision": "r0-open", "tag": "t", "bundle_sha256": "abc", "manifest_sha256": "def", "numeric_policy_version": "p"}"#,
        "not json",
    ] {
        assert!(Pin::parse(text.as_bytes()).is_err(), "{text}");
    }
}
