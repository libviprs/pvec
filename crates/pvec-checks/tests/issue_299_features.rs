//! Issue 299: the feature table. Every feature has a golden that exercises it, the generated markdown is the
//! committed one, `pvec-scene::SUPPORTED` is the driver column, and every region of PAINT.md chapter 7 is
//! claimed by at least one feature.

use pvec_capabilities::{Reader, Registry, Version, ids};
use pvec_checks::*;
use pvec_components::{
    ClipStream, FillPaintStream, FillRule, GradientCoords, ItemKind, PaintOrderStream, Stream,
    StrokePaintStream, read_streams,
};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use std::fs;

fn table() -> Vec<Feature> {
    let text = fs::read_to_string(repo_root().join("docs/features.toml")).unwrap();
    parse_features(&text).unwrap()
}

fn streams_of(golden: &str) -> Vec<Stream> {
    let bytes = fs::read(repo_root().join(golden)).unwrap_or_else(|e| panic!("{golden}: {e}"));
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let at = reader
        .entries()
        .iter()
        .position(|e| e.block_kind() == Some(BlockKind::Components))
        .unwrap_or_else(|| panic!("{golden}: no component block"));
    let set = reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec();
    let current = Reader {
        version: Version::new(1, 3, 0),
        implemented: vec![ids::PAINT_ORDER, ids::STROKE_PAINT, ids::FILL_PAINT, ids::CLIP],
    };
    read_streams(&set, &Registry::v1(), &current).unwrap().streams
}

/// Whether the decoded streams of the golden really use the feature, not merely contain the stream.
fn exercised(key: &str, streams: &[Stream]) -> bool {
    let payload = |id| streams.iter().find(|s| s.capability == id).map(|s| s.payload.as_slice());
    let order = || payload(ids::PAINT_ORDER).map(|p| PaintOrderStream::decode(p).unwrap());
    let strokes = || payload(ids::STROKE_PAINT).map(|p| StrokePaintStream::decode(p).unwrap());
    let fills = || payload(ids::FILL_PAINT).map(|p| FillPaintStream::decode(p).unwrap());
    let clips = || payload(ids::CLIP).map(|p| ClipStream::decode(p).unwrap());
    let kind = |k: ItemKind| order().is_some_and(|o| o.runs.iter().any(|r| r.kind == k));
    let rule = |r: FillRule| fills().is_some_and(|f| f.assignments.iter().any(|a| a.rule == r));
    let gradient = |axial: bool| {
        fills().is_some_and(|f| {
            f.gradients.iter().any(|g| matches!(g.coords, GradientCoords::Axial { .. }) == axial)
        })
    };
    match key {
        "23.stroke_item" => kind(ItemKind::Stroke),
        "23.fill_item" => kind(ItemKind::Fill),
        "23.raster_item" => kind(ItemKind::Raster),
        "23.text_item" => kind(ItemKind::Text),
        "24.colour" => strokes().is_some_and(|s| !s.assignments.is_empty()),
        "24.alpha" => strokes().is_some_and(|s| s.colours.iter().any(|c| c & 0xFF != 0xFF)),
        "25.rule.nonzero" => rule(FillRule::NonZero),
        "25.rule.evenodd" => rule(FillRule::EvenOdd),
        "25.gradient.axial" => gradient(true),
        "25.gradient.radial" => gradient(false),
        "25.gradient.extend" => {
            fills().is_some_and(|f| f.gradients.iter().any(|g| g.extend_before || g.extend_after))
        }
        "25.tiling.coloured" => fills().is_some_and(|f| f.tilings.iter().any(|t| t.coloured)),
        "25.tiling.uncoloured" => fills().is_some_and(|f| f.tilings.iter().any(|t| !t.coloured)),
        "26.clip" => clips().is_some_and(|c| !c.clips.is_empty() && !c.runs.is_empty()),
        "26.nested" => clips().is_some_and(|c| c.clips.iter().any(|r| r.parent.is_some())),
        other => panic!("{other} has no probe in this test; add one when adding the feature"),
    }
}

#[test]
fn the_table_has_the_right_shape() {
    let t = table();
    check_features(&t).unwrap();
    for k in [
        "23.text_item",
        "23.raster_item",
        "24.alpha",
        "25.gradient.axial",
        "25.gradient.radial",
        "25.tiling.coloured",
        "25.rule.evenodd",
        "none.blend_mode",
        "none.shading.mesh",
    ] {
        assert!(t.iter().any(|f| f.key == k), "{k} is missing");
    }
}

#[test]
fn the_shape_rules_refuse_a_feature_with_no_golden_and_a_duplicate_key() {
    let mut t = table();
    let i = t.iter().position(|f| f.key == "25.tiling.coloured").unwrap();
    t[i].golden = None;
    assert!(check_features(&t).unwrap_err().contains("no golden exercises it"));
    let mut t = table();
    t.push(t[0].clone());
    assert!(check_features(&t).unwrap_err().contains("listed twice"));
}

#[test]
fn every_golden_exercises_its_feature() {
    let missed: Vec<String> = table()
        .iter()
        .filter(|f| f.golden.is_some())
        .filter(|f| !exercised(&f.key, &streams_of(f.golden.as_deref().unwrap())))
        .map(|f| f.key.clone())
        .collect();
    assert!(missed.is_empty(), "no golden exercises {missed:?}");
}

#[test]
fn a_probe_notices_a_golden_of_the_wrong_stream() {
    let streams = streams_of("tests/compat/blueprint-mix-23-paint-order.pvec");
    assert!(!exercised("25.gradient.axial", &streams));
}

#[test]
fn the_committed_features_document_is_exactly_the_generated_one() {
    let path = repo_root().join("docs/FEATURES.md");
    let generated = render_features_markdown(&table());
    if std::env::var_os("PVEC_BLESS_FEATURES").is_some() {
        fs::write(&path, &generated).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/FEATURES.md differs from docs/features.toml; regenerate it with PVEC_BLESS_FEATURES=1 cargo \
         test -p pvec-checks --test issue_299_features"
    );
    let mut changed = generated.clone().into_bytes();
    changed[10] ^= 1;
    assert!(changed != committed.into_bytes(), "a single changed byte must be seen");
}

#[test]
fn scene_supported_is_the_driver_column() {
    let mut from_table: Vec<&str> = Vec::new();
    let t = table();
    for f in t.iter().filter(|f| f.driver) {
        from_table.push(&f.key);
    }
    assert_eq!(pvec_scene::SUPPORTED, from_table.as_slice());
}

#[test]
fn every_paint_md_region_is_claimed_by_a_feature() {
    let regions = paint_regions(&fs::read_to_string(repo_root().join("docs/PAINT.md")).unwrap());
    assert_eq!(regions.len(), 45, "chapter 7 lists 45 regions: {regions:?}");
    let t = table();
    for r in &regions {
        assert!(t.iter().any(|f| f.regions.contains(r)), "{r} maps to no feature");
    }
    for f in &t {
        for r in &f.regions {
            assert!(regions.contains(r), "{}: {r} is not a region of PAINT.md chapter 7", f.key);
        }
    }
}

#[test]
fn the_region_reader_expands_a_bare_id_with_the_fixture_before_it() {
    let md = "## 7. x\n\n| 2 | fid-a/one, two; fid-b/three | c | d |\n## 8. y\n| 1 | fid-z/no | c | d |\n";
    assert_eq!(paint_regions(md), ["fid-a/one", "fid-a/two", "fid-b/three"]);
}
