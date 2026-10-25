//! Issue #119: `layer-state-001` is generated, and the layer stream in the artifact carries the
//! on and frozen state of each layer the record names.

mod common;

use common::*;
use pvec_capabilities::ids;
use pvec_components::{LAYER_FROZEN, LAYER_OFF, LayerStream};
use pvec_testkit::generate::generate_all;
use pvec_testkit::reference::read;

#[test]
fn issue_119_layer_state_001_is_generated_with_its_state_in_the_layer_stream() {
    let outcomes = generate_all(&bundle()).unwrap();
    let o = outcomes.into_iter().find(|o| o.fixture == "layer-state-001").unwrap();
    let artifact = o.result.unwrap_or_else(|r| panic!("still refused: {r:?}"));
    let opened = read(&artifact.bytes).unwrap();
    assert!(opened.stream_capabilities.contains(&ids::LAYER.get()));
    let s = opened.streams.iter().find(|s| s.capability == ids::LAYER).expect("a layer stream");
    assert_eq!(s.version.major, 2);
    let layers = LayerStream::decode(&s.payload).unwrap();
    let state = |n: &str| layers.flags[layers.names.iter().position(|x| x == n).unwrap()];
    assert_eq!(state("SHOWN_L"), 0);
    assert_eq!(state("OFF_L"), LAYER_OFF);
    assert_eq!(state("FROZEN_L"), LAYER_FROZEN);
    // each of the three lines is on its own layer, and none moved to another one
    assert_eq!(layers.assignments.len(), 3);
    let layer_of = |entity: &str| {
        let id = artifact.ids.iter().find(|(n, _)| n.ends_with(entity)).unwrap().1;
        let (_, i) = layers.assignments.iter().find(|(e, _)| e.index() == id).unwrap();
        layers.names[*i as usize].clone()
    };
    assert_eq!(layer_of(".shown-l"), "SHOWN_L");
    assert_eq!(layer_of(".off-l"), "OFF_L");
    assert_eq!(layer_of(".frozen-l"), "FROZEN_L");
}

#[test]
fn issue_119_a_record_without_layers_writes_no_layer_stream() {
    let outcomes = generate_all(&bundle()).unwrap();
    let o = outcomes.into_iter().find(|o| o.fixture == "line-001").unwrap();
    let opened = read(&o.result.unwrap().bytes).unwrap();
    assert!(!opened.stream_capabilities.contains(&ids::LAYER.get()));
}

#[test]
fn issue_119_the_artifact_is_still_byte_identical_across_runs() {
    let bytes = || {
        let o = generate_all(&bundle()).unwrap();
        o.into_iter().find(|o| o.fixture == "layer-state-001").unwrap().result.map(|a| a.bytes)
    };
    assert_eq!(bytes().ok(), bytes().ok());
    assert!(bytes().is_ok(), "layer-state-001 is not generated");
}
