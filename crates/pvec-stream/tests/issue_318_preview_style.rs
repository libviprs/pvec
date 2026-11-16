//! Issue pvec#318: a Tier 0 preview draws as the style of the entity it replaces.
//!
//! The style is not in the leaf stream and is not added to it: the component streams (14, 15, 24) are
//! keyed by `EntityId`, and a preview's only identity is `replaces`, so the id is the style key. These
//! tests pin the rule and pin that the stream did not grow.
#![allow(missing_docs)]

use pvec_components::{
    LAYER_OFF, LayerStream, LineStyleDef, LineStyleStream, StrokePaintStream, StrokeWidth,
};
use pvec_entity::EntityId;
use pvec_stream::{Preview, PreviewForm, Tier0Builder, Tier0Params, Tier0Plan};

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

fn preview(id: u32) -> Preview {
    Preview::new(eid(id), PreviewForm::ChordPolyline, 0.01, [10.0, 20.0], vec![[0.0, 0.0], [1.0, 2.0]]).unwrap()
}

fn lines() -> LineStyleStream {
    LineStyleStream {
        styles: vec![
            LineStyleDef { name: "fill only".into(), dashes: vec![], width: None, phase: 0.0, visible: false },
            LineStyleDef {
                name: "hidden dashed".into(),
                dashes: vec![2.0, 1.0],
                width: Some(StrokeWidth::Width(0.5)),
                phase: 0.0,
                visible: true,
            },
        ],
        assignments: vec![(eid(5), 0), (eid(7), 1)],
    }
}

fn layers() -> LayerStream {
    LayerStream { names: vec!["A-WALL".into(), "A-OFF".into()], flags: vec![0, LAYER_OFF], assignments: vec![(eid(5), 1), (eid(7), 0)] }
}

fn paint() -> StrokePaintStream {
    StrokePaintStream { colours: vec![0xFF00_00FF], assignments: vec![(eid(5), 0)] }
}

#[test]
fn a_preview_takes_the_line_style_of_its_entity() {
    let (l, y, p) = (lines(), layers(), paint());
    let hidden = preview(5).style(Some(&l), Some(&y), Some(&p));
    assert!(!hidden.line_style.expect("entity 5 has a line style").visible);
    let dashed = preview(7).style(Some(&l), Some(&y), Some(&p));
    let def = dashed.line_style.expect("entity 7 has a line style");
    assert_eq!(def.dashes, vec![2.0, 1.0]);
    assert_eq!(def.width, Some(StrokeWidth::Width(0.5)));
}

#[test]
fn a_preview_takes_the_layer_flags_and_colour_of_its_entity() {
    let (l, y, p) = (lines(), layers(), paint());
    let s = preview(5).style(Some(&l), Some(&y), Some(&p));
    assert_eq!(s.layer_flags, LAYER_OFF);
    assert_eq!(s.colour, Some(0xFF00_00FF));
    let t = preview(7).style(Some(&l), Some(&y), Some(&p));
    assert_eq!((t.layer_flags, t.colour), (0, None));
}

#[test]
fn an_entity_with_no_assignment_and_a_file_with_no_stream_draw_as_the_default() {
    let (l, y, p) = (lines(), layers(), paint());
    let plain = preview(9).style(Some(&l), Some(&y), Some(&p));
    let none = preview(5).style(None, None, None);
    for s in [plain, none] {
        assert_eq!((s.line_style, s.layer_flags, s.colour), (None, 0, None));
    }
}

#[test]
fn the_style_survives_the_leaf_stream_and_the_stream_carries_none() {
    let mut b = Tier0Builder::new(8, Tier0Params { max_error: 1.0 }).unwrap();
    for i in 0..8 {
        if i == 5 {
            b.add_preview(preview(5)).unwrap();
        } else {
            b.add_exact(eid(i)).unwrap();
        }
    }
    let bytes = b.finish().unwrap().encode();
    // header 24, two exact runs (0 to 4 and 6 to 7) of 8, one preview of id 4, form 1, count 2, bound 4,
    // origin 16 and two vertices of 8: no style field.
    assert_eq!(bytes.len(), 24 + 2 * 8 + (4 + 1 + 2 + 4 + 16 + 2 * 8));
    let back = Tier0Plan::decode(&bytes).unwrap();
    let (l, y, p) = (lines(), layers(), paint());
    let s = back.previews()[0].style(Some(&l), Some(&y), Some(&p));
    assert!(!s.line_style.unwrap().visible);
    assert_eq!(s.colour, Some(0xFF00_00FF));
}
