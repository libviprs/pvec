//! Issue #259: the record types the exported component streams hold are reachable through the
//! facade, so a consumer can fill a stream without naming a physical crate.

use pvec::EntityId;
use pvec::components::{
    ElementClass, ElementEntry, LAYER_FROZEN, LAYER_LOCKED, LAYER_OFF, LineStyleDef, LineStyleStream, Origin,
    ProvenanceRecord, ProvenanceStream, RegionKind, RegionRecord, SemanticClassStream, SourceTag, StrokeWidth,
};

#[test]
fn issue_259_a_consumer_fills_the_line_style_provenance_and_semantic_streams() {
    let e = |n| EntityId::new(n).unwrap();
    let styles = LineStyleStream {
        styles: vec![LineStyleDef {
            name: "dash".into(),
            dashes: vec![1.0, 2.0],
            width: Some(StrokeWidth::Width(0.5)),
            phase: 0.25,
            visible: true,
        }],
        assignments: vec![(e(0), 0)],
    };
    assert_eq!(LineStyleStream::decode(&styles.encode().unwrap()).unwrap(), styles);
    let prov = ProvenanceStream { records: vec![ProvenanceRecord { entity: e(0), source_kind: 1, handle: vec![1, 2] }] };
    assert_eq!(ProvenanceStream::decode(&prov.encode().unwrap()).unwrap(), prov);
    let sem = SemanticClassStream {
        elements: vec![ElementEntry {
            entity: e(0),
            class: ElementClass::Wall,
            origin: Origin { source: SourceTag::Explicit, confidence: None },
        }],
        regions: vec![RegionRecord {
            kind: RegionKind::Room,
            origin: Origin { source: SourceTag::Explicit, confidence: None },
            outer: vec![e(0)],
            holes: vec![],
        }],
    };
    assert_eq!(SemanticClassStream::decode(&sem.encode().unwrap()).unwrap(), sem);
    assert_eq!([LAYER_OFF, LAYER_FROZEN, LAYER_LOCKED], [1, 2, 4]);
}
