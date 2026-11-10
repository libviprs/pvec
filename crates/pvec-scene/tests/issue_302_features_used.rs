//! Issue 302: `features_used` reads the feature keys from the decoded streams of a file, so the libviprs gate
//! has no second list to drift. Over the goldens, and over a stream that cannot be decoded.

use pvec_capabilities::{Reader, Registry, Version, ids};
use pvec_components::{Stream, read_streams};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_scene::features_used;
use std::fs;
use std::path::Path;

fn streams_of(golden: &str) -> Vec<Stream> {
    let bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat").join(golden)).unwrap();
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let at = reader.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Components)).unwrap();
    let set = reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec();
    let current = Reader {
        version: Version::new(1, 3, 0),
        implemented: vec![ids::PAINT_ORDER, ids::STROKE_PAINT, ids::FILL_PAINT, ids::CLIP],
    };
    read_streams(&set, &Registry::v1(), &current).unwrap().streams
}

#[test]
fn a_paint_order_golden_names_its_four_item_kinds_and_nothing_else() {
    let used = features_used(&streams_of("blueprint-mix-23-paint-order.pvec"));
    assert_eq!(used, ["23.stroke_item", "23.fill_item", "23.raster_item", "23.text_item"]);
}

#[test]
fn a_fill_golden_names_gradients_and_tilings_it_holds() {
    let used = features_used(&streams_of("blueprint-mix-25-radial-tiling.pvec"));
    assert!(used.contains(&"25.gradient.radial") && used.contains(&"25.tiling.coloured"), "{used:?}");
    assert!(!used.contains(&"26.clip"), "{used:?}");
}

#[test]
fn a_clip_golden_names_clips_and_nesting() {
    let used = features_used(&streams_of("blueprint-mix-26-clip.pvec"));
    assert!(used.contains(&"26.clip") && used.contains(&"26.nested"), "{used:?}");
}

#[test]
fn a_stream_that_cannot_be_decoded_contributes_nothing() {
    let mut streams = streams_of("blueprint-mix-26-clip.pvec");
    for s in &mut streams {
        s.payload = vec![0xFF; 3];
    }
    assert!(features_used(&streams).is_empty());
}
