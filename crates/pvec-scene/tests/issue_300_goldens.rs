//! Issue 300: the expected plan of each blueprint-mix compat file, written next to it and compared byte for byte.
//! pvec, libviprs and planviewer read the same files and the same JSON, so a driver that drifts fails in all
//! three. The format is in `docs/PAINT.md` section 5.3. `PVEC_BLESS=1` rewrites the files from the driver.

use pvec_capabilities::{Reader, Registry, Version, ids};
use pvec_components::{Band, FillRule, ItemKind, Stream, read_streams};
use pvec_container::directory::BlockKind;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_entity::EntityId;
use pvec_scene::{ClipTree, Counts, Degrade, Policy, ScenePlan};
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

const FILES: [&str; 5] = [
    "blueprint-mix-23-paint-order",
    "blueprint-mix-24-stroke-paint",
    "blueprint-mix-25-fill-paint",
    "blueprint-mix-25-radial-tiling",
    "blueprint-mix-26-clip",
];

fn compat(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat").join(name)
}

fn streams_of(stem: &str) -> Vec<Stream> {
    let bytes = fs::read(compat(&format!("{stem}.pvec"))).expect("the golden file");
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let at = reader
        .entries()
        .iter()
        .position(|e| e.block_kind() == Some(BlockKind::Components))
        .expect("a component block");
    let set = reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec();
    let current = Reader {
        version: Version::new(1, 3, 0),
        implemented: vec![ids::PAINT_ORDER, ids::STROKE_PAINT, ids::FILL_PAINT, ids::CLIP],
    };
    read_streams(&set, &Registry::v1(), &current).unwrap().streams
}

/// The page every blueprint-mix file describes: eight strokes, three fills, one text record, one underlay raster.
fn page() -> Counts {
    Counts { strokes: 8, fills: 3, texts: 1, rasters: vec![Band::Underlay] }
}

fn kind(k: ItemKind) -> &'static str {
    match k {
        ItemKind::Stroke => "stroke",
        ItemKind::Fill => "fill",
        ItemKind::Raster => "raster",
        ItemKind::Text => "text",
    }
}

/// The expected plan as text: fixed key order, one object per line, no floats, a final newline. `degraded` is the
/// variant with a batch limit of 2 and no ring for any clip; the other has the limit 4,096 and a unit square.
fn plan_json(stem: &str, degraded: bool) -> String {
    let counts = page();
    let policy = Policy { max_batches: if degraded { 2 } else { 4096 } };
    let ring_of = |_: EntityId| (!degraded).then(|| vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    let streams = streams_of(stem);
    let plan = ScenePlan::build(&streams, &counts, policy);
    let tree = ClipTree::build(&streams, &plan, ring_of);
    let mut s = String::new();
    let bands: Vec<String> = counts
        .rasters
        .iter()
        .map(|b| format!("\"{}\"", if *b == Band::Underlay { "underlay" } else { "overlay" }))
        .collect();
    writeln!(
        s,
        "{{\n  \"counts\": {{\"strokes\": {}, \"fills\": {}, \"texts\": {}, \"rasters\": [{}]}},",
        counts.strokes,
        counts.fills,
        counts.texts,
        bands.join(", ")
    )
    .unwrap();
    writeln!(s, "  \"policy\": {{\"max_batches\": {}}},", policy.max_batches).unwrap();
    writeln!(s, "  \"rings\": \"{}\",", if degraded { "none" } else { "unit_square" }).unwrap();
    let array = |name: &str, lines: Vec<String>| {
        if lines.is_empty() {
            format!("  \"{name}\": [],\n")
        } else {
            format!("  \"{name}\": [\n{}\n  ],\n", lines.join(",\n"))
        }
    };
    let lines: Vec<String> = plan
        .batches
        .iter()
        .map(|b| {
            format!("    {{\"kind\": \"{}\", \"first\": {}, \"count\": {}}}", kind(b.kind), b.first, b.count)
        })
        .collect();
    s.push_str(&array("batches", lines));
    let lines: Vec<String> = tree
        .nodes
        .iter()
        .map(|n| {
            format!(
                "    {{\"rule\": \"{}\", \"parent\": {}, \"depth\": {}, \"write_bit\": {}, \"chain_mask\": {}, \"rings\": {}}}",
                if n.rule == FillRule::NonZero { "nonzero" } else { "evenodd" },
                n.parent.map_or("null".to_string(), |p| p.to_string()),
                n.depth,
                n.write_bit,
                n.chain_mask,
                n.rings.as_ref().map_or("null".to_string(), |r| r.len().to_string()),
            )
        })
        .collect();
    s.push_str(&array("clip_nodes", lines));
    let lines: Vec<String> = tree
        .runs
        .iter()
        .map(|r| {
            format!(
                "    {{\"batch\": {}, \"first\": {}, \"count\": {}, \"clip\": {}}}",
                r.batch,
                r.first,
                r.count,
                r.clip.map_or("null".to_string(), |c| c.to_string())
            )
        })
        .collect();
    s.push_str(&array("clip_runs", lines));
    let degrades: Vec<String> = plan
        .degrades
        .iter()
        .chain(&tree.degrades)
        .map(|d| match d {
            Degrade::TooManyBatches => "\"too_many_batches\"".to_string(),
            Degrade::TilingTooLarge => "\"tiling_too_large\"".to_string(),
            Degrade::StreamDamaged { id, rule } => format!("\"stream_damaged:{}:{rule}\"", id.get()),
            Degrade::ClipUnbuildable { clip, items } => format!("\"clip_unbuildable:{clip}:{items}\""),
        })
        .collect();
    writeln!(s, "  \"degrades\": [{}]\n}}", degrades.join(", ")).unwrap();
    s
}

fn check(stem: &str, degraded: bool) {
    let name = format!("{stem}{}.plan.json", if degraded { ".degraded" } else { "" });
    let got = plan_json(stem, degraded);
    if std::env::var_os("PVEC_BLESS").is_some() {
        fs::write(compat(&name), &got).unwrap();
    }
    let want = fs::read_to_string(compat(&name)).unwrap_or_else(|_| panic!("tests/compat/{name} is missing"));
    assert_eq!(got, want, "tests/compat/{name} differs from what the driver gives");
}

#[test]
fn each_file_gives_the_expected_plan_byte_for_byte() {
    for stem in FILES {
        check(stem, false);
    }
}

#[test]
fn each_file_gives_the_expected_degrades_byte_for_byte() {
    for stem in FILES {
        check(stem, true);
    }
}

#[test]
fn the_goldens_carry_what_each_stream_is_for() {
    let order = plan_json("blueprint-mix-23-paint-order", false);
    assert!(order.contains("\"degrades\": []"), "{order}");
    assert!(order.contains("{\"kind\": \"fill\", \"first\": 0, \"count\": 1}"));
    let clip = plan_json("blueprint-mix-26-clip", false);
    assert!(clip.contains("\"depth\": 2") && clip.contains("\"chain_mask\": 1"), "{clip}");
    let none = plan_json("blueprint-mix-26-clip", true);
    assert!(none.contains("clip_unbuildable:"), "{none}");
    let capped = plan_json("blueprint-mix-23-paint-order", true);
    assert!(capped.contains("too_many_batches") || capped.contains("stream_damaged"), "{capped}");
}

#[test]
fn the_goldens_are_ordinary_text_with_a_final_newline() {
    for stem in FILES {
        for name in [format!("{stem}.plan.json"), format!("{stem}.degraded.plan.json")] {
            let text = fs::read_to_string(compat(&name)).unwrap_or_else(|_| panic!("{name} is missing"));
            assert!(text.ends_with("}\n") && !text.contains('\r') && !text.contains('\t'), "{name}");
        }
    }
}

#[test]
fn the_scene_plan_fuzz_target_is_named_in_the_docs_and_the_script() {
    let root = compat("../..");
    let read = |p: &str| fs::read_to_string(root.join(p)).unwrap_or_else(|_| panic!("{p}"));
    let doc = read("docs/FUZZING.md");
    assert!(doc.contains("`fuzz_scene_plan` is the fourteenth") && doc.contains("| `fuzz_scene_plan` |"));
    assert!(read("tools/fuzz.sh").contains("fuzz_scene_plan"));
    assert!(read("fuzz/fuzz_targets/fuzz_scene_plan.rs").contains("pvec_fuzz::fuzz_scene_plan"));
    assert!(read("fuzz/Cargo.toml").contains("name = \"fuzz_scene_plan\""));
}
