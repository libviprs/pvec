//! The valid artifacts every case is a mutation of.

use super::mutate::Layout;
use crate::generate::{GENERATOR_VERSION, PRODUCER_PREFIX, generate_all_with, generate_record_with};
use crate::records::*;
use pvec_capabilities::Registry;
use pvec_components::LayerStream;
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::compress::{Codec, CodecPolicy};
use pvec_container::integrity::Integrity;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig, WriterOptions};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_corpus::Bundle;
use pvec_entity::EntityId;

/// What the parse-tier bases are written with: no checksum table, no digest and no compression. A case
/// that changes a byte of a block has to reach the parser that is meant to refuse it, and with a checksum
/// in the file every such edit would be refused by the checksum first, which would hide a reader that
/// forgot to call the parser. The integrity cases have their own sealed bases.
pub const PARSE: WriterOptions = WriterOptions { integrity: Integrity::None, codecs: CodecPolicy::NONE };

/// Checksums only: the per-block table and no artifact digest.
pub const CHECKSUMS: WriterOptions =
    WriterOptions { integrity: Integrity::Checksums, codecs: CodecPolicy::NONE };

/// Both the table and the digest.
pub const SEALED: WriterOptions = WriterOptions { integrity: Integrity::Full, codecs: CodecPolicy::NONE };

/// One codec for every kind of block, nothing sealed.
pub fn packed(codec: Codec) -> WriterOptions {
    WriterOptions { integrity: Integrity::None, codecs: CodecPolicy::uniform(codec) }
}

/// A named valid artifact.
#[derive(Clone, Debug)]
pub struct Base {
    /// What it is: a corpus fixture id, or one of the four synthetic names.
    pub name: String,
    /// The bytes of a valid artifact.
    pub bytes: Vec<u8>,
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).expect("a small id")
}

fn writer(config: WriterConfig) -> ContainerWriter {
    ContainerWriter::with_options(config, PARSE)
}

fn config(name: &str) -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: format!("{PRODUCER_PREFIX}{name}"),
            producer_version: GENERATOR_VERSION.to_string(),
            corpus_revision: Some("hostile-r1".to_string()),
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn meta(i: u32) -> EntityMeta {
    EntityMeta { id: id(i), style: 0, cell: 0, bounds: Bounds { min: [0.0, 0.0], max: [8.0, 8.0] } }
}

/// Composite curves nested to the deepest the container allows (NP-18: depth 4), a composite that
/// nothing references (so a case can lengthen a chain without touching what already works), and one
/// with trims. No corpus record maps onto a composite yet (the bulge fixtures are refused, #101), so
/// this one is written through the writer directly.
fn composites() -> Vec<u8> {
    let mut w = writer(config("composites"));
    let line = |a: f64| [0.0, 0.0 + a, 2.0, 1.0 + a];
    for (n, a) in [(10u32, 0.0), (11, 2.0)] {
        let p = line(a);
        let spec = SplineSpec {
            degree: 1,
            knots: &[0.0, 0.0, 1.0, 1.0],
            x: &[p[0], p[2]],
            y: &[p[1], p[3]],
            weights: None,
            closed: false,
            periodic: false,
        };
        w.push_spline(meta(n), spec).expect("a spline");
    }
    type Composite<'a> = (u32, &'a [u32], Option<&'a [[f64; 2]]>);
    let composites: [Composite; 5] = [
        (20, &[10, 11], None),
        (21, &[20, 11], None),
        (22, &[21, 10], None),
        (23, &[11, 10], None),
        (24, &[11, 11], Some(&[[0.0, 1.0], [0.0, 1.0]])),
    ];
    for (n, segs, trims) in composites {
        let ids: Vec<EntityId> = segs.iter().map(|s| id(*s)).collect();
        w.push_composite(meta(n), &ids, trims, false).expect("a composite");
    }
    w.finish().expect("the composites base writes")
}

/// A line and a layer stream, so there is a component block, which is optional.
fn streams() -> Vec<u8> {
    let mut w = writer(config("streams"));
    w.push_line(meta(0), [0.0, 0.0], [3.0, 4.0]).expect("a line");
    let layers =
        LayerStream { names: vec!["WALLS".to_string(), "DOORS".to_string()], assignments: vec![(id(0), 1)] };
    w.push_stream(layers.into_stream(&Registry::v1()).expect("a layer stream")).expect("a stream");
    w.finish().expect("the streams base writes")
}

fn from_record(name: &str, entities: &[String]) -> Base {
    from_record_with(name, name, entities, PARSE)
}

fn from_record_with(name: &str, record_name: &str, entities: &[String], options: WriterOptions) -> Base {
    let math = record(record_name, entities);
    let art = generate_record_with(&math, "hostile-r1", options).unwrap_or_else(|e| panic!("{name}: {e}"));
    Base { name: name.to_string(), bytes: art.bytes }
}

/// Every hot block kind and both advanced kinds in one file.
fn drawing() -> Base {
    drawing_with("drawing", PARSE)
}

fn drawing_with(name: &str, options: WriterOptions) -> Base {
    let mut es = Vec::new();
    for i in 0..5 {
        let y = f64::from(i) * 1.5;
        es.push(line(&format!("l{i}"), [0.0, y, 0.0], [3.0 + f64::from(i), y + 0.5, 0.0]));
    }
    es.push(circle("c0", [4.0, 4.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 1.25));
    es.push(arc("a0", [1.0, 1.0, 0.0], 2.5, 0.5, 1.5));
    es.push(ellipse("e0", [2.0, 1.0, 0.0], [1.0, 0.0, 0.0], 3.0, 2.0));
    es.push(polyline("p0", &[[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 1.0, 0.0]], &[0.0, 0.0, 0.0], false));
    es.push(polyline(
        "p1",
        &[[5.0, 5.0, 0.0], [6.0, 7.0, 0.0], [8.0, 6.0, 0.0], [9.0, 9.0, 0.0]],
        &[0.0; 4],
        false,
    ));
    es.push(bspline(
        "s0",
        2,
        &[[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [2.0, -1.0, 0.0], [3.0, 1.0, 0.0]],
        &[(0.0, 3), (0.5, 1), (1.0, 3)],
        None,
        OPEN,
    ));
    es.push(bspline(
        "s1",
        2,
        &[[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [2.0, -1.0, 0.0], [3.0, 1.0, 0.0]],
        &[(0.0, 3), (0.5, 1), (1.0, 3)],
        Some(&[1.0, 0.5, 2.0, 1.0]),
        OPEN,
    ));
    from_record_with(name, "drawing", &es, options)
}

/// Splines with distinct knot vectors: a degree 2 curve with five interior knots (so a case can
/// repeat them), a rational degree 3 curve, a Bezier and a closed curve.
fn splines() -> Base {
    let p = |i: f64| [i, (i * 7.0) % 5.0 * 0.75, 0.0];
    let eight: Vec<[f64; 3]> = (0..8).map(|i| p(f64::from(i))).collect();
    let seven: Vec<[f64; 3]> = (0..7).map(|i| p(f64::from(i) + 0.5)).collect();
    let closed: Vec<[f64; 3]> =
        [[0.0, 0.0], [2.0, 3.0], [5.0, 3.5], [6.0, 1.0], [4.0, -2.0], [1.0, -2.0], [0.0, 0.0]]
            .iter()
            .map(|q| [q[0], q[1], 0.0])
            .collect();
    let es = [
        bspline(
            "deg2",
            2,
            &eight,
            &[(0.0, 3), (0.125, 1), (0.25, 1), (0.5, 1), (0.625, 1), (0.75, 1), (1.0, 3)],
            None,
            OPEN,
        ),
        bspline(
            "rational",
            3,
            &seven,
            &[(0.0, 4), (0.25, 1), (0.5, 1), (0.75, 1), (1.0, 4)],
            Some(&[1.0, 0.75, 1.5, 2.0, 1.25, 0.5, 1.0]),
            OPEN,
        ),
        bspline("closed", 3, &closed, &[(0.0, 4), (0.25, 1), (0.5, 1), (0.75, 1), (1.0, 4)], None, CLOSED),
    ];
    from_record("splines", &es)
}

/// `drawing` with a legal gap of eight zero bytes before its second block and another before the
/// directory. The writer leaves none (every block is a multiple of 8 long), so without this base the
/// padding rules of the reader could never be reached.
fn gapped(drawing: &[u8]) -> Vec<u8> {
    let l = Layout::of(drawing);
    let (b1, dir) = (l.blocks[1].offset, l.directory);
    let mut v = drawing[..b1].to_vec();
    v.extend_from_slice(&[0; 8]);
    v.extend_from_slice(&drawing[b1..dir]);
    v.extend_from_slice(&[0; 8]);
    v.extend_from_slice(&drawing[dir..]);
    let new_dir = dir + 16;
    // the pointer to the directory, and the offset of every block after the first
    let ptr = l.pointer_table + pvec_container::spec::dir::OFFSET;
    v[ptr..ptr + 8].copy_from_slice(&(new_dir as u64).to_le_bytes());
    for (i, b) in l.blocks.iter().enumerate().skip(1) {
        let at = new_dir + i * pvec_container::directory::ENTRY_LEN + entry_offset();
        v[at..at + 8].copy_from_slice(&((b.offset + 8) as u64).to_le_bytes());
    }
    v
}

fn entry_offset() -> usize {
    super::mutate::entry_field("offset")
}

/// Every base: the emitted corpus fixtures by id (parse tier), then `drawing`, `splines`, `composites`,
/// `streams` and `gapped`, then `line-001+checksums`, `line-001+sealed`, `drawing+lz4`, `drawing+deflate`
/// and `drawing+lz4+sealed` for the integrity and decompression cases.
pub fn bases(bundle: &Bundle) -> Vec<Base> {
    let emitted = |options: WriterOptions| -> Vec<Base> {
        generate_all_with(bundle, options)
            .expect("the bundle registers")
            .into_iter()
            .filter_map(|o| o.result.ok().map(|a| Base { name: a.fixture_id, bytes: a.bytes }))
            .collect()
    };
    let mut out: Vec<Base> = emitted(PARSE);
    let line = |all: Vec<Base>, suffix: &str| {
        let b = all.into_iter().find(|b| b.name == "line-001").expect("line-001 is emitted");
        Base { name: format!("line-001{suffix}"), bytes: b.bytes }
    };
    let (line_sums, line_sealed) = (line(emitted(CHECKSUMS), "+checksums"), line(emitted(SEALED), "+sealed"));
    out.push(drawing());
    out.push(splines());
    out.push(Base { name: "composites".to_string(), bytes: composites() });
    out.push(Base { name: "streams".to_string(), bytes: streams() });
    let gap = gapped(&out.iter().find(|b| b.name == "drawing").expect("drawing").bytes);
    out.push(Base { name: "gapped".to_string(), bytes: gap });
    // the integrity and the compression bases: the same drawing and the same line, sealed, and packed
    out.push(line_sums);
    out.push(line_sealed);
    out.push(drawing_with("drawing+lz4", packed(Codec::Lz4)));
    out.push(drawing_with("drawing+deflate", packed(Codec::Deflate)));
    out.push(drawing_with(
        "drawing+lz4+sealed",
        WriterOptions { integrity: Integrity::Full, codecs: CodecPolicy::uniform(Codec::Lz4) },
    ));
    out
}
