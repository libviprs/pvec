//! `docs/CONTAINER.md` chapter 1 and the constants the code reads (issue P4.1).

use pvec_container::spec::*;
use pvec_container::{Header, Manifest};
use std::fs;
use std::path::PathBuf;

fn doc_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/CONTAINER.md")
}

fn contiguous(fields: &[FieldSpec], total: usize) {
    assert!(!fields.is_empty());
    let mut at = 0;
    for f in fields {
        assert_eq!(f.offset, at, "{} starts at {} but the previous field ends at {at}", f.name, f.offset);
        assert!(f.size > 0, "{}", f.name);
        at += f.size;
    }
    assert_eq!(at, total, "the fields add up to {at} bytes, not {total}");
}

#[test]
fn the_header_table_covers_exactly_the_fixed_header_with_no_gaps() {
    contiguous(HEADER_FIELDS, Header::LEN);
}

#[test]
fn the_manifest_table_covers_exactly_the_fixed_part_with_no_gaps() {
    contiguous(MANIFEST_FIELDS, Manifest::FIXED_LEN);
}

#[test]
fn the_entry_tables_cover_their_entries() {
    contiguous(CAPABILITY_ENTRY_FIELDS, 16);
    contiguous(DIRECTORY_ENTRY_FIELDS, 24);
}

#[test]
fn the_tables_name_the_fields_the_issue_requires() {
    let names: Vec<&str> = HEADER_FIELDS.iter().map(|f| f.name).collect();
    for needed in [
        "magic",
        "container_major",
        "container_minor",
        "byte_order",
        "header_len",
        "manifest_offset",
        "manifest_length",
        "directory_table_offset",
        "directory_table_length",
        "block_checksum",
        "artifact_checksum",
    ] {
        assert!(names.contains(&needed), "header table lacks {needed}: {names:?}");
    }
    let names: Vec<&str> = MANIFEST_FIELDS.iter().map(|f| f.name).collect();
    for needed in [
        "length_unit",
        "metres_per_unit",
        "min_x",
        "min_y",
        "max_x",
        "max_y",
        "capability_count",
        "directory_count",
        "block_checksums_offset",
        "artifact_digest",
        "producer_id_len",
        "producer_version_len",
        "corpus_revision_len",
    ] {
        assert!(names.contains(&needed), "manifest table lacks {needed}: {names:?}");
    }
}

#[test]
fn the_committed_container_document_is_exactly_the_generated_one() {
    let generated = render_markdown();
    if std::env::var_os("PVEC_BLESS_CONTAINER").is_some() {
        fs::write(doc_path(), &generated).unwrap();
    }
    let committed = fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/CONTAINER.md differs from the constants; regenerate it with PVEC_BLESS_CONTAINER=1 \
         cargo test -p pvec-container --test container_doc"
    );
}

#[test]
fn a_changed_offset_or_size_in_the_document_is_detected() {
    let generated = render_markdown();
    assert!(!generated.is_empty());
    let f = &HEADER_FIELDS[2];
    let row_offset = format!("| {} |", f.offset);
    assert!(generated.contains(&row_offset), "the document has no row for offset {}", f.offset);
    let tampered = generated.replacen(&row_offset, &format!("| {} |", f.offset + 1), 1);
    assert_ne!(tampered, generated);
    // the equality the previous test makes is the detector
    assert!(fs::read_to_string(doc_path()).unwrap_or_default() != tampered);
}

#[test]
fn every_field_name_and_offset_appears_in_the_document() {
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(!HEADER_FIELDS.is_empty() && !MANIFEST_FIELDS.is_empty());
    for f in HEADER_FIELDS
        .iter()
        .chain(MANIFEST_FIELDS)
        .chain(CAPABILITY_ENTRY_FIELDS)
        .chain(DIRECTORY_ENTRY_FIELDS)
    {
        let row = format!("| {} | {} | {} |", f.offset, f.size, f.name);
        assert!(doc.contains(&row), "docs/CONTAINER.md has no row {row:?}");
    }
}

#[test]
fn chapter_one_states_byte_order_reserved_space_and_growth_rules() {
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(doc.starts_with("# "), "the document has a title");
    for needle in
        ["## 1. The container prefix", "Byte order", "Reserved space", "Growth rules", "little-endian"]
    {
        assert!(doc.contains(needle), "docs/CONTAINER.md lacks {needle:?}");
    }
}

#[test]
fn the_hot_header_table_is_in_the_document_row_for_row() {
    use pvec_container::blocks::hot::{HOT_HEADER_FIELDS, HOT_HEADER_LEN};
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    contiguous(HOT_HEADER_FIELDS, HOT_HEADER_LEN);
    for f in HOT_HEADER_FIELDS {
        let row = format!("| {} | {} | {} |", f.offset, f.size, f.name);
        assert!(doc.contains(&row), "docs/CONTAINER.md has no hot block row {row:?}");
    }
}

#[test]
fn chapter_two_states_the_alignment_and_padding_rules_and_every_kind() {
    use pvec_container::blocks::hot::HotKind;
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    for needle in [
        "## 2. Hot geometry blocks",
        "natural alignment",
        "Padding",
        "WASM linear memory",
        "ascending `EntityId`",
        "run offsets",
    ] {
        assert!(doc.contains(needle), "docs/CONTAINER.md lacks {needle:?}");
    }
    for k in HotKind::ALL {
        let row = format!("| {} |", k.id());
        assert!(doc.contains(&row), "no row for hot kind {k:?}");
        for c in k.f64_columns() {
            assert!(doc.contains(c), "kind {k:?}: no mention of column {c}");
        }
    }
}

#[test]
fn every_hot_block_rule_is_documented_once_and_the_error_type_reports_them() {
    use pvec_container::blocks::hot::{HOT_RULES, HotError};
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    for (rule, meaning) in HOT_RULES {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/CONTAINER.md lacks {rule}");
    }
    let mut ids: Vec<&str> = HOT_RULES.iter().map(|r| r.0).collect();
    ids.dedup();
    assert_eq!(ids.len(), HOT_RULES.len(), "a rule id is listed twice");
    for e in [HotError::BadMagic, HotError::NonZeroPadding, HotError::Misaligned, HotError::EmptyBlock] {
        assert!(ids.contains(&e.rule()), "{e} reports a rule the document does not list");
    }
}

fn chapter_three_tables() -> [(&'static str, &'static [FieldSpec], usize); 3] {
    use pvec_container::blocks::advanced::*;
    use pvec_container::blocks::arena::*;
    [
        ("arena header", ARENA_HEADER_FIELDS, ARENA_HEADER_LEN),
        ("record block header", RECORD_HEADER_FIELDS, RECORD_HEADER_LEN),
        ("record", RECORD_FIELDS, RECORD_LEN),
    ]
}

#[test]
fn the_arena_and_record_tables_cover_their_structures_with_no_gaps() {
    for (name, fields, len) in chapter_three_tables() {
        assert!(!fields.is_empty(), "{name}: no fields");
        contiguous(fields, len);
    }
}

#[test]
fn every_arena_and_record_field_appears_in_the_document_row_for_row() {
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    for (name, fields, _) in chapter_three_tables() {
        assert!(!fields.is_empty(), "{name}: no fields");
        for f in fields {
            let row = format!("| {} | {} | {} |", f.offset, f.size, f.name);
            assert!(doc.contains(&row), "docs/CONTAINER.md has no {name} row {row:?}");
        }
    }
}

#[test]
fn chapter_three_states_the_layout_the_invariants_and_every_payload_kind() {
    use pvec_container::blocks::arena::PayloadKind;
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    for needle in [
        "## 3. Advanced geometry arenas",
        "### 3.1",
        "offset table",
        "A reader may assume",
        "A reader must verify",
        "natural alignment",
        "two integer reads",
        "no nested per-entity byte blob",
        "periodic",
        "nesting",
        "share",
    ] {
        assert!(doc.contains(needle), "docs/CONTAINER.md lacks {needle:?}");
    }
    for k in PayloadKind::ALL {
        let row = format!("| {} | {} |", k.id(), k.name());
        assert!(doc.contains(&row), "no row for payload kind {k:?}");
        for c in k.columns() {
            assert!(doc.contains(c), "kind {k:?}: no mention of column {c}");
        }
    }
}

#[test]
fn every_arena_rule_is_documented_once_and_the_error_type_reports_them() {
    use pvec_container::blocks::arena::{ARENA_RULES, ArenaError};
    use pvec_numeric::NumericError;
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    for (rule, meaning) in ARENA_RULES {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/CONTAINER.md lacks {rule}");
    }
    let mut ids: Vec<&str> = ARENA_RULES.iter().map(|r| r.0).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "a rule id is listed twice");
    assert!(before >= 20, "the rule table is suspiciously short: {before}");
    // one value of every variant must report a listed rule
    let all = [
        ArenaError::Truncated { needed: 1, got: 0 },
        ArenaError::BadMagic,
        ArenaError::UnsupportedVersion { found: 2 },
        ArenaError::UnknownKind { id: 9 },
        ArenaError::NonZeroReserved,
        ArenaError::NonZeroPadding,
        ArenaError::LengthMismatch { declared: 1, actual: 2 },
        ArenaError::Misaligned,
        ArenaError::HostEndianness,
        ArenaError::EntryOutOfRange { index: 1, entries: 0 },
        ArenaError::RangeOutOfArena { start: 1, len: 1, arena_len: 1 },
        ArenaError::WrongPayload { held: pvec_container::blocks::arena::PayloadKind::Knots },
        ArenaError::KnotCount { knots: 1, control_points: 1, degree: 1 },
        ArenaError::WeightCount { weights: 1, control_points: 2 },
        ArenaError::ControlPointColumns { x: 1, y: 2 },
        ArenaError::PeriodicUnsupported,
        ArenaError::InvalidFlags { bits: 8 },
        ArenaError::RecordFields { index: 0 },
        ArenaError::IdsNotAscending { index: 1 },
        ArenaError::SegmentOutOfRange { record: 0, segment: 9 },
        ArenaError::CompositeCycle { record: 0 },
        ArenaError::NestingTooDeep { record: 0 },
        ArenaError::TrimCount { values: 1, segments: 1 },
        ArenaError::Numeric(NumericError::BadKnots),
        ArenaError::EmptyBlock,
        ArenaError::DuplicateId { id: 1 },
        ArenaError::UnknownSegment { id: 1 },
        ArenaError::TooLarge,
    ];
    for e in all {
        assert!(ids.contains(&e.rule()), "{e} reports a rule the document does not list");
    }
}

// ---- chapter 4: the writer (issue P4.4) ----

#[test]
fn the_block_entry_table_covers_its_entry_and_is_in_the_document_row_for_row() {
    use pvec_container::directory::{BLOCK_ENTRY_FIELDS, ENTRY_LEN};
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    contiguous(BLOCK_ENTRY_FIELDS, ENTRY_LEN);
    for f in BLOCK_ENTRY_FIELDS {
        let row = format!("| {} | {} | {} |", f.offset, f.size, f.name);
        assert!(doc.contains(&row), "docs/CONTAINER.md has no block entry row {row:?}");
    }
}

#[test]
fn chapter_four_states_the_layout_the_canonical_order_and_the_metadata_rules() {
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    for needle in [
        "## 4. The writer",
        "### 4.1 File layout",
        "### 4.2 The block directory",
        "### 4.3 Canonical order",
        "### 4.4 Stable metadata",
        "### 4.5 Refusals",
        "### 4.6 Review checklist",
        "block kind",
        "spatial cell id",
        "geometry type",
        "`EntityId`",
        "capability id",
        "stream id",
        "little-endian",
        "byte-identical",
    ] {
        assert!(doc.contains(needle), "docs/CONTAINER.md lacks {needle:?}");
    }
    use pvec_container::directory::BlockKind;
    for k in BlockKind::ALL {
        assert!(doc.contains(&format!("| {} | {} |", k.id(), k.name())), "no row for block kind {k:?}");
    }
}

#[test]
fn every_writer_and_directory_rule_is_documented_once_and_the_errors_report_them() {
    use pvec_container::directory::DIRECTORY_RULES;
    use pvec_container::writer::{WRITE_RULES, WriteError};
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(WRITE_RULES.len() >= 13, "{} writer rules listed", WRITE_RULES.len());
    for (rule, meaning) in WRITE_RULES.iter().chain(DIRECTORY_RULES) {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/CONTAINER.md lacks {rule}");
    }
    let mut ids: Vec<&str> = WRITE_RULES.iter().map(|r| r.0).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "a writer rule id is listed twice");
    for e in [
        WriteError::Empty,
        WriteError::DuplicateId { id: 1 },
        WriteError::CellOnAdvanced { id: 1 },
        WriteError::UnknownCapability { id: 1 },
        WriteError::DuplicateStream { capability: 1 },
        WriteError::BadBounds { id: None },
        WriteError::TooLarge,
        WriteError::Unsupported { what: "x" },
    ] {
        assert!(ids.contains(&e.rule()), "{e} reports {}, which is not listed", e.rule());
    }
}

#[test]
fn chapter_four_says_that_cell_zero_is_both_no_cell_and_a_cell() {
    let doc = fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(doc.contains("cell 0 is both"), "docs/CONTAINER.md does not say that cell 0 means two things");
    assert!(doc.contains("P5.1"), "the cell 0 note does not point at the spatial directory");
}
