//! The limits against the phase 2 corpus (issue P4.8): the defaults admit every fixture, and
//! `docs/SECURITY_LIMITS.md` lists what the browser preset refuses.

mod common;

use common::bundle;
use pvec_container::limits::{self, Limits};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_testkit::hostile::bases;
use std::path::PathBuf;

fn doc_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/SECURITY_LIMITS.md")
}

fn reads(bytes: &[u8], limits: Limits) -> bool {
    let cfg = ReaderConfig { limits, ..ReaderConfig::default() };
    ContainerReader::open(MemorySource::new(bytes.to_vec()), cfg).and_then(|r| r.validate()).is_ok()
}

#[test]
fn the_default_limits_admit_every_phase_2_corpus_fixture() {
    let all = bases(&bundle());
    assert!(all.len() >= 20, "{} bases", all.len());
    for b in &all {
        assert!(reads(&b.bytes, Limits::DEFAULT), "the default limits refuse {}", b.name);
    }
}

fn browser_rejects() -> Vec<String> {
    bases(&bundle()).iter().filter(|b| !reads(&b.bytes, Limits::BROWSER)).map(|b| b.name.clone()).collect()
}

#[test]
fn the_committed_limits_document_is_exactly_the_generated_one_and_lists_the_browser_rejections() {
    let generated = limits::render_markdown(&browser_rejects());
    if std::env::var_os("PVEC_BLESS_LIMITS").is_some() {
        std::fs::write(doc_path(), &generated).unwrap();
    }
    let committed = std::fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/SECURITY_LIMITS.md differs from limits.rs; regenerate it with PVEC_BLESS_LIMITS=1 \
         cargo test -p pvec-testkit --test issue_9_limits_corpus"
    );
    for kind in limits::LimitKind::ALL {
        assert!(committed.contains(kind.rule()), "the document does not mention {}", kind.rule());
    }
}

#[test]
fn the_committed_document_lists_the_four_raster_ceilings_with_their_values() {
    let committed = std::fs::read_to_string(doc_path()).unwrap_or_default();
    for needle in [
        "| LIMIT-13 | Rasters | `max_rasters` | 4096 | 4096 |",
        "| LIMIT-14 | RasterTiles | `max_raster_tiles` | 1048576 | 131072 |",
        "| LIMIT-15 | TileBytes | `max_tile_bytes` | 65536 | 65536 |",
        "| LIMIT-16 | PlanDecodedBytes | `max_plan_decoded_bytes` | 268435456 | 268435456 |",
    ] {
        assert!(committed.contains(needle), "docs/SECURITY_LIMITS.md has no row {needle}");
    }
}
