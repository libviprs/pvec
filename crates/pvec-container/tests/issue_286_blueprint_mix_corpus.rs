//! Issue 286 follow-up: one golden file per paint stream (23 to 26), written with the semantics of the
//! `blueprint-mix` page (a sheet frame, a filled title block, a raster underlay with a white mask and a translucent
//! hatch over it, strokes in two colours, a title text, and a drawing area clip with a hole). Each file holds the
//! geometry and that one stream. A current reader gets the stream back unchanged and skips nothing, and an old
//! reader (no capability from 14 up) opens the file with the geometry intact, skips the stream as not implemented
//! and never reads its payload. `PVEC_BLESS_FIXTURE=1` writes the files again.

use pvec_capabilities::{CapabilityId, Reader, Registry, Version, ids};
use pvec_components::*;
use pvec_container::directory::BlockKind;
use pvec_container::negotiate::{ReaderCapabilities, SkipReason};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use std::fs;
use std::path::Path;

fn id(n: u32) -> EntityId {
    EntityId::new(n).expect("an id")
}

/// The page: eight closed or open polylines in points, y up. 0 sheet frame, 1 title block outline, 2 drawing line,
/// 3 dimension line, 4 and 5 and 6 the rings of three fills (title block, white mask, hatch), 7 the clip ring.
fn page() -> [(u32, Vec<[f64; 2]>); 8] {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
    [
        (0, rect(18.0, 18.0, 594.0, 774.0)),
        (1, rect(400.0, 18.0, 594.0, 90.0)),
        (2, vec![[40.0, 400.0], [300.0, 520.0], [500.0, 400.0]]),
        (3, vec![[40.0, 120.0], [540.0, 120.0]]),
        (4, rect(400.0, 18.0, 594.0, 90.0)),
        (5, rect(60.0, 200.0, 360.0, 330.0)),
        (6, rect(200.0, 260.0, 540.0, 600.0)),
        (7, rect(36.0, 110.0, 576.0, 760.0)),
    ]
}

fn file_with(stream: Stream) -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "pvec-blueprint-mix-corpus".into(),
            producer_version: "0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    for (n, pts) in page() {
        let min = [
            pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min),
            pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
        ];
        let max = [
            pts.iter().map(|p| p[0]).fold(f64::MIN, f64::max),
            pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max),
        ];
        w.push_polyline(EntityMeta { id: id(n), style: 0, cell: 0, bounds: Bounds { min, max } }, &pts)
            .expect("a polyline");
    }
    w.push_stream(stream).expect("the stream");
    w.finish().expect("the file")
}

fn paint_order() -> PaintOrderStream {
    let run = |kind, first, count| PaintRun { kind, first, count };
    PaintOrderStream {
        runs: vec![
            run(ItemKind::Fill, 0, 1),
            run(ItemKind::Raster, 0, 1),
            run(ItemKind::Fill, 1, 2),
            run(ItemKind::Stroke, 0, 4),
            run(ItemKind::Text, 0, 1),
        ],
    }
}

fn stroke_paint() -> StrokePaintStream {
    StrokePaintStream {
        colours: vec![0x0000_00FF, 0xC800_00FF, 0x0000_FF80],
        assignments: vec![(id(0), 0), (id(1), 0), (id(2), 1), (id(3), 2)],
    }
}

fn fill_paint() -> FillPaintStream {
    FillPaintStream {
        gradients: vec![Gradient {
            coords: GradientCoords::Axial { x0: 400.0, y0: 18.0, x1: 594.0, y1: 18.0 },
            extend_before: true,
            extend_after: true,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            stops: vec![Stop { offset: 0.0, rgba: 0xE0E0_E0FF }, Stop { offset: 1.0, rgba: 0xFFFF_FFFF }],
        }],
        tilings: vec![Tiling {
            coloured: false,
            x_step: 6.0,
            y_step: 6.0,
            cell: [0.0, 0.0, 6.0, 6.0],
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            primitives: vec![TilingPrimitive::Polyline {
                rgba: 0,
                width: 0.5,
                points: vec![[0.0, 0.0], [6.0, 6.0]],
            }],
        }],
        assignments: vec![
            FillAssignment { hatch: 0, rule: FillRule::NonZero, paint: FillSource::Gradient(0) },
            FillAssignment { hatch: 1, rule: FillRule::NonZero, paint: FillSource::Own },
            FillAssignment { hatch: 2, rule: FillRule::EvenOdd, paint: FillSource::Tiling(0) },
        ],
    }
}

/// Issue 299: the two paints the blueprint-mix page does not use, a radial cone with no extension and a coloured
/// tiling of a filled polygon, so each feature of `docs/features.toml` has a golden that exercises it.
fn fill_paint_variants() -> FillPaintStream {
    FillPaintStream {
        gradients: vec![Gradient {
            coords: GradientCoords::Radial { x0: 120.0, y0: 240.0, r0: 4.0, x1: 180.0, y1: 280.0, r1: 60.0 },
            extend_before: false,
            extend_after: false,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            stops: vec![Stop { offset: 0.0, rgba: 0xFF00_00FF }, Stop { offset: 1.0, rgba: 0x0000_FFFF }],
        }],
        tilings: vec![Tiling {
            coloured: true,
            x_step: 8.0,
            y_step: 8.0,
            cell: [0.0, 0.0, 8.0, 8.0],
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            primitives: vec![TilingPrimitive::Polygon {
                rule: FillRule::NonZero,
                rgba: 0x2080_20FF,
                points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            }],
        }],
        assignments: vec![
            FillAssignment { hatch: 0, rule: FillRule::NonZero, paint: FillSource::Gradient(0) },
            FillAssignment { hatch: 1, rule: FillRule::NonZero, paint: FillSource::Tiling(0) },
        ],
    }
}

fn clip() -> ClipStream {
    ClipStream {
        clips: vec![
            ClipRecord { rule: FillRule::NonZero, parent: None, boundary: vec![id(7)] },
            ClipRecord { rule: FillRule::EvenOdd, parent: Some(0), boundary: vec![id(6), id(5)] },
        ],
        runs: vec![
            ClipRun { kind: ItemKind::Raster, first: 0, count: 1, clip: 0 },
            ClipRun { kind: ItemKind::Fill, first: 1, count: 2, clip: 1 },
            ClipRun { kind: ItemKind::Stroke, first: 2, count: 2, clip: 0 },
        ],
    }
}

fn cases() -> Vec<(&'static str, CapabilityId, Stream)> {
    let r = Registry::v1();
    vec![
        ("blueprint-mix-23-paint-order", ids::PAINT_ORDER, paint_order().into_stream(&r).unwrap()),
        ("blueprint-mix-24-stroke-paint", ids::STROKE_PAINT, stroke_paint().into_stream(&r).unwrap()),
        ("blueprint-mix-25-fill-paint", ids::FILL_PAINT, fill_paint().into_stream(&r).unwrap()),
        ("blueprint-mix-25-radial-tiling", ids::FILL_PAINT, fill_paint_variants().into_stream(&r).unwrap()),
        ("blueprint-mix-26-clip", ids::CLIP, clip().into_stream(&r).unwrap()),
    ]
}

fn geometry(bytes: &[u8], caps: ReaderCapabilities) -> Vec<Vec<u8>> {
    let cfg = ReaderConfig { capabilities: caps, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(bytes.to_vec()), cfg).expect("the file opens");
    let mut out = Vec::new();
    for (i, e) in reader.entries().iter().enumerate() {
        if matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)) {
            out.push(reader.read_block(BlockId(i as u32)).expect("a geometry block").as_bytes().to_vec());
        }
    }
    out
}

fn component_set(bytes: &[u8]) -> Vec<u8> {
    let reader = ContainerReader::open(MemorySource::new(bytes.to_vec()), ReaderConfig::default()).unwrap();
    let at = reader
        .entries()
        .iter()
        .position(|e| e.block_kind() == Some(BlockKind::Components))
        .expect("a component block");
    reader.read_block(BlockId(at as u32)).unwrap().as_bytes().to_vec()
}

fn old_reader() -> ReaderCapabilities {
    let mut r = (14..=26).fold(ReaderCapabilities::v1(), |r, n| r.without(CapabilityId::new(n)));
    r.version = Version::new(1, 0, 0);
    r
}

#[test]
fn the_golden_files_are_what_this_test_writes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat");
    for (name, _, stream) in cases() {
        let want = file_with(stream);
        let path = dir.join(format!("{name}.pvec"));
        if std::env::var_os("PVEC_BLESS_FIXTURE").is_some() {
            fs::write(&path, &want).unwrap();
        }
        let have =
            fs::read(&path).unwrap_or_else(|_| panic!("{name}.pvec: write it with PVEC_BLESS_FIXTURE=1"));
        assert!(have == want, "{name}.pvec is not what this test writes: bless it again");
    }
}

#[test]
fn a_current_reader_gets_each_stream_back_and_skips_nothing() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat");
    let registry = Registry::v1();
    let current = Reader {
        version: Version::new(1, 3, 0),
        implemented: vec![ids::PAINT_ORDER, ids::STROKE_PAINT, ids::FILL_PAINT, ids::CLIP],
    };
    for (name, cap, stream) in cases() {
        let bytes = fs::read(dir.join(format!("{name}.pvec"))).unwrap();
        let cfg = ReaderConfig::default();
        let reader = ContainerReader::open(MemorySource::new(bytes.clone()), cfg).unwrap();
        assert!(reader.partial_open().is_none(), "{name}: nothing is skipped");
        let opened = read_streams(&component_set(&bytes), &registry, &current).unwrap();
        assert!(opened.skipped.is_empty(), "{name}");
        assert_eq!(opened.streams.len(), 1, "{name}");
        assert_eq!(
            (opened.streams[0].capability, &opened.streams[0].payload),
            (cap, &stream.payload),
            "{name}"
        );
    }
}

#[test]
fn an_old_reader_skips_each_stream_unread_and_keeps_the_geometry() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat");
    let registry = Registry::v1();
    let old = Reader { version: Version::new(1, 0, 0), implemented: Vec::new() };
    let base =
        geometry(&fs::read(dir.join("blueprint-mix-23-paint-order.pvec")).unwrap(), ReaderCapabilities::v1());
    for (name, cap, _) in cases() {
        let bytes = fs::read(dir.join(format!("{name}.pvec"))).unwrap();
        let cfg = ReaderConfig { capabilities: old_reader(), ..ReaderConfig::default() };
        let reader =
            ContainerReader::open(MemorySource::new(bytes.clone()), cfg).expect("an old reader opens it");
        let report = reader.partial_open().expect("a report");
        assert_eq!(report.capabilities.len(), 1, "{name}");
        assert_eq!(
            (report.capabilities[0].id, report.capabilities[0].reason),
            (cap, SkipReason::NotImplemented),
            "{name}"
        );
        assert_eq!(geometry(&bytes, old_reader()), base, "{name}: the geometry is the same");
        let opened = read_streams(&component_set(&bytes), &registry, &old).unwrap();
        assert!(opened.streams.is_empty(), "{name}");
        assert_eq!(opened.skipped.len(), 1, "{name}");
        assert_eq!(opened.skipped[0].reason, pvec_components::SkipReason::NotImplemented, "{name}");
    }
}

#[test]
fn each_stream_validates_against_the_page() {
    let drawn = |e: EntityId| e.index() < 8;
    paint_order().validate(drawn, 3, 1, 1).unwrap();
    clip().validate(drawn, drawn, 3, 1, 1).unwrap();
    fill_paint().validate(3).unwrap();
    fill_paint_variants().validate(3).unwrap();
}
