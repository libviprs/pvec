//! pvec#146: docs/ISO_PROFILE.md must describe what the STEP writer emits for a composite curve's
//! `self_intersect` logical. The writer says `.U.` (unknown) because PVec never computes
//! self-intersection, so the document may not claim `.F.`.

use pvec_interchange::step::*;

mod common;
use common::*;

fn doc() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ISO_PROFILE.md")).unwrap()
}

fn composite_row(doc: &str) -> String {
    doc.lines()
        .find(|l| l.contains("COMPOSITE_CURVE('',"))
        .expect("the composite_curve row of docs/ISO_PROFILE.md has no Part 21 form")
        .to_string()
}

fn written_self_intersect() -> String {
    let e = StepEntity::exact(eid(COMPOSITE), composite());
    let out = export(&[e], &StepOptions::new("c.stp", "2026-10-24T12:00:00").unwrap()).unwrap();
    let line = out.part21.lines().find(|l| l.contains("=COMPOSITE_CURVE(")).expect("no composite written");
    let tail = line.trim_end_matches(");");
    tail.rsplit(',').next().unwrap().to_string()
}

#[test]
fn issue_146_the_writer_emits_unknown_for_a_composite() {
    assert_eq!(written_self_intersect(), ".U.");
}

#[test]
fn issue_146_the_doc_shows_the_logical_the_writer_emits() {
    let row = composite_row(&doc());
    let want = format!("COMPOSITE_CURVE('',(#s0,#s1,...),{});", written_self_intersect());
    assert!(row.contains(&want), "ISO_PROFILE.md composite form disagrees with the writer: {row}");
}

#[test]
fn issue_146_the_doc_says_why_it_does_not_claim_false() {
    let row = composite_row(&doc());
    assert!(
        row.contains("self_intersect") && row.contains("does not compute"),
        "the composite row must explain that self-intersection is not computed, so .F. is not claimed"
    );
}
