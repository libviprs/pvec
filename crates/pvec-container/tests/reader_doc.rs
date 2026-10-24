//! `docs/READER.md` is generated from the constants of the reader (issue P4.5).

use pvec_container::reader::{READER_RULES, render_markdown};
use std::path::PathBuf;

fn doc_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/READER.md")
}

#[test]
fn the_committed_reader_document_is_exactly_the_generated_one() {
    let generated = render_markdown();
    assert!(!generated.is_empty(), "the document is not generated yet");
    if std::env::var_os("PVEC_BLESS_READER").is_some() {
        std::fs::write(doc_path(), &generated).unwrap();
    }
    let committed = std::fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/READER.md differs from the reader; regenerate it with PVEC_BLESS_READER=1 \
         cargo test -p pvec-container --test reader_doc"
    );
}

#[test]
fn every_rule_the_reader_adds_to_the_parsers_is_in_the_document() {
    let doc = std::fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(!READER_RULES.is_empty());
    for (rule, _) in READER_RULES {
        assert!(doc.contains(rule), "docs/READER.md does not mention {rule}");
    }
    for needle in ["Opening a file", "The read plan", "The cache", "never panics"] {
        assert!(doc.contains(needle), "docs/READER.md lacks {needle:?}");
    }
}
