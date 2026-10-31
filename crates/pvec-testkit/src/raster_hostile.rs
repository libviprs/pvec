//! The hostile tiles (issue #219): thirteen tiles a producer, or an attacker, can hand a container, each with
//! the rule that must refuse it, and the accepted variants beside them. A reader that serves tiles to a host
//! decoder is held to this list: each hostile tile is refused by name, the file still opens and its geometry is
//! intact, and no refused tile can be fetched as an image.
//!
//! The container's writer refuses what the reader would, so a hostile tile cannot be written the ordinary way.
//! [`file`] writes it with the writer's tile check switched off, which is the one use that switch has.

use crate::tilegen;
use pvec_capabilities::Version;
use pvec_components::{
    Band, PixelFormat, Placement, RASTER_CAPABILITY, RasterId, Stream, TileCodec, encode_rasters,
    encode_streams,
};
use pvec_container::blocks::raster::TileIn;
use pvec_container::blocks::sniff::{JPEG_WALK, max_tile_bytes};
use pvec_container::directory::{BlockKind, FLAG_HAS_BOUNDS, FLAG_MUST_UNDERSTAND};
use pvec_container::integrity::Integrity;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, RawBlock, WriterConfig, assemble};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;

/// The part of `LIMIT-15` that does not depend on the edge, as the default limits have it.
pub const ALLOWANCE: u32 = 65_536;

/// One tile and what a reader must do with it.
#[derive(Clone, Debug)]
pub struct Tile {
    /// A name, `kebab-case`.
    pub name: &'static str,
    /// The codec id of the placement it is under (1 PNG, 2 JPEG, 3 WebP, anything else an unknown codec).
    pub codec: u8,
    /// The tile edge of the placement.
    pub edge: u16,
    /// 1 gray, 2 gray and alpha, 3 RGB, 4 RGBA.
    pub channels: u8,
    /// 1 or 8.
    pub depth: u8,
    /// The bytes of the tile.
    pub bytes: Vec<u8>,
    /// The rule that refuses it (`RASTER-03`, `RASTER-04` or `LIMIT-15`), or `None` for a tile that passes.
    pub rule: Option<&'static str>,
    /// Why, in a sentence.
    pub why: &'static str,
}

fn tile(
    name: &'static str,
    (codec, channels, depth): (u8, u8, u8),
    bytes: Vec<u8>,
    rule: &'static str,
    why: &'static str,
) -> Tile {
    Tile { name, codec, edge: 256, channels, depth, bytes, rule: Some(rule), why }
}

/// The thirteen hostile tiles, all under a placement of tiles of 256.
pub fn hostile() -> Vec<Tile> {
    let d = tilegen::payload(400, 5);
    let ceiling = max_tile_bytes(256, ALLOWANCE) as usize;
    let at_ceiling = |extra: usize| {
        let header = tilegen::png(256, 1, 8, &[]).len();
        tilegen::png(256, 1, 8, &tilegen::payload(ceiling - header + extra, 1))
    };
    let mut polyglot = tilegen::png(256, 1, 8, &d);
    polyglot.extend_from_slice(b"PK\x03\x04\x14\x00\x00\x00\x08\x00 a zip archive after the image");
    vec![
        tile(
            "oversized-tile",
            (1, 1, 8),
            tilegen::png(256, 1, 8, &tilegen::payload(2_000_000, 1)),
            "LIMIT-15",
            "a tile far over twice the bytes of an uncompressed RGBA tile, whose only purpose is decode time",
        ),
        tile(
            "polyglot-png-and-zip",
            (1, 1, 8),
            polyglot,
            "RASTER-03",
            "a valid PNG followed by an archive: bytes after IEND, a tile that is also something else",
        ),
        tile(
            "animated-webp",
            (3, 4, 8),
            tilegen::webp_vp8x(256, 256, true, true, &d),
            "RASTER-03",
            "a WebP with the animation flag: a decoder would take a frame count from the file",
        ),
        tile(
            "interlaced-png",
            (1, 1, 8),
            tilegen::png_with(256, 256, 8, 0, 1, &d, 1),
            "RASTER-03",
            "an interlaced PNG, whose decode needs seven passes the placement does not account for",
        ),
        tile(
            "sixteen-bit-png",
            (1, 1, 8),
            tilegen::png_with(256, 256, 16, 0, 0, &d, 1),
            "RASTER-03",
            "a 16-bit PNG under a placement of 8 bits",
        ),
        tile(
            "palette-png",
            (1, 3, 8),
            tilegen::png_with(256, 256, 8, 3, 0, &d, 1),
            "RASTER-03",
            "a palette PNG, which carries no channel the placement names",
        ),
        tile(
            "jpeg-with-four-components",
            (2, 3, 8),
            tilegen::jpeg(256, 4, false, &d),
            "RASTER-03",
            "a CMYK JPEG under an RGB placement",
        ),
        tile(
            "jpeg-frame-header-past-64-kib",
            (2, 3, 8),
            tilegen::jpeg_with(256, 256, 3, false, JPEG_WALK + 4_000, &d, true),
            "RASTER-03",
            "a JPEG whose frame header is past the walk the sniffer is allowed",
        ),
        tile(
            "header-lies-about-the-size",
            (1, 1, 8),
            tilegen::png_with(255, 256, 8, 0, 0, &d, 1),
            "RASTER-03",
            "a PNG that says 255 by 256 under a placement of 256 by 256",
        ),
        tile(
            "bit-depth-mismatch",
            (1, 1, 1),
            tilegen::png(256, 1, 8, &d),
            "RASTER-03",
            "an 8-bit PNG under a placement of 1 bit",
        ),
        tile(
            "raw-tile",
            (0, 1, 8),
            vec![0u8; 256 * 256],
            "RASTER-04",
            "raw pixels carry neither dimensions nor a pixel format, so a Raw tile is not a tile",
        ),
        tile(
            "jpeg-under-an-rgba-placement",
            (2, 4, 8),
            tilegen::jpeg(256, 3, false, &d),
            "RASTER-04",
            "JPEG has no alpha, so the pair is refused before a byte of the tile is read",
        ),
        tile(
            "tile-at-limit-15-plus-one",
            (1, 1, 8),
            at_ceiling(1),
            "LIMIT-15",
            "a well-formed PNG one byte over the ceiling of a tile",
        ),
    ]
}

/// Tiles that pass, one of each codec and variant libviprs writes, at an edge of 256.
pub fn accepted() -> Vec<Tile> {
    let d = tilegen::payload(400, 7);
    let ok = |name, (codec, channels, depth), bytes| Tile {
        name,
        codec,
        edge: 256,
        channels,
        depth,
        bytes,
        rule: None,
        why: "a tile libviprs writes",
    };
    vec![
        ok("png-gray-1-bit", (1, 1, 1), tilegen::png(256, 1, 1, &d)),
        ok("png-gray-8-bit", (1, 1, 8), tilegen::png(256, 1, 8, &d)),
        ok("png-gray-alpha", (1, 2, 8), tilegen::png(256, 2, 8, &d)),
        ok("png-rgb", (1, 3, 8), tilegen::png(256, 3, 8, &d)),
        ok("png-rgba", (1, 4, 8), tilegen::png(256, 4, 8, &d)),
        ok("jpeg-gray-baseline", (2, 1, 8), tilegen::jpeg(256, 1, false, &d)),
        ok("jpeg-gray-progressive", (2, 1, 8), tilegen::jpeg(256, 1, true, &d)),
        ok("jpeg-rgb-baseline", (2, 3, 8), tilegen::jpeg(256, 3, false, &d)),
        ok("jpeg-rgb-progressive", (2, 3, 8), tilegen::jpeg(256, 3, true, &d)),
        ok("webp-vp8", (3, 3, 8), tilegen::webp_vp8(256, 256, &d)),
        ok("webp-vp8l", (3, 4, 8), tilegen::webp_vp8l(256, 256, &d)),
        ok("webp-vp8x", (3, 4, 8), tilegen::webp_vp8x(256, 256, false, true, &d)),
    ]
}

/// The placement a tile is under: a 256 by 256 image in one tile, so one level.
pub fn placement(t: &Tile) -> Placement {
    Placement {
        pixel_to_source: [1.0, 0.0, 0.0, -1.0, 0.0, 256.0],
        width: 256,
        height: 256,
        visible_box: [0, 0, 256, 256],
        tile_edge: t.edge,
        levels: 1,
        tile_codec: TileCodec::from_word(t.codec),
        pixel_format: match t.channels {
            1 => PixelFormat::Gray,
            2 => PixelFormat::GrayAlpha,
            3 => PixelFormat::Rgb,
            _ => PixelFormat::Rgba,
        },
        bit_depth: t.depth,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }
}

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-hostile-tiles".into(),
            producer_version: "0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

/// A file with one line of geometry and one raster holding the tile, written with the tile check off. A placement
/// the stream's own rules refuse (JPEG under RGBA) is written as a valid one with its codec byte changed after the
/// stream is encoded, since the stream encoder refuses it too.
pub fn file(t: &Tile) -> Vec<u8> {
    let mut p = placement(t);
    let refused_pair = t.codec == 2 && !matches!(t.channels, 1 | 3);
    if refused_pair {
        p.tile_codec = TileCodec::Png;
    }
    let mut w = ContainerWriter::new(config());
    let bounds = Bounds { min: [0.0, 0.0], max: [10.0, 10.0] };
    w.push_line(
        EntityMeta { id: EntityId::new(0).unwrap(), style: 0, cell: 0, bounds },
        [0.0, 0.0],
        [10.0, 10.0],
    )
    .unwrap();
    w.push_stream(Stream {
        capability: RASTER_CAPABILITY,
        version: Version::new(1, 1, 0),
        payload: encode_rasters(std::slice::from_ref(&p)).expect("a placement the rules accept"),
    })
    .unwrap();
    w.skip_tile_checks();
    w.push_raster_tiles(
        RasterId::new(0).unwrap(),
        vec![TileIn { level: 0, col: 0, row: 0, bytes: t.bytes.clone() }],
    )
    .unwrap();
    let bytes = w.finish().expect("the hostile tile is written");
    if refused_pair { with_codec(&bytes, t.codec) } else { bytes }
}

/// `file` laid out again with the first placement's codec byte set to `codec`, sealed as before.
fn with_codec(file: &[u8], codec: u8) -> Vec<u8> {
    let r = ContainerReader::open(MemorySource::new(file.to_vec()), ReaderConfig::default()).unwrap();
    let mut blocks: Vec<RawBlock> = Vec::new();
    for (i, e) in r.entries().iter().enumerate() {
        let mut stored = r.read_stored(BlockId(i as u32)).unwrap().as_slice().to_vec();
        let (mut codec_id, mut uncompressed_length) = (e.codec, e.uncompressed_length);
        if e.block_kind() == Some(BlockKind::Components) {
            let decoded = r.read_block(BlockId(i as u32)).unwrap();
            let opened = pvec_components::read_streams(
                decoded.as_bytes(),
                &pvec_capabilities::Registry::v1(),
                &pvec_capabilities::Reader {
                    version: Version::new(1, 1, 0),
                    implemented: pvec_capabilities::Registry::v1().entries().iter().map(|c| c.id).collect(),
                },
            )
            .unwrap();
            let mut s = opened.streams[0].clone();
            // the count, then the fixed part of the first record: the codec is its byte 75
            s.payload[4 + 75] = codec;
            stored = encode_streams(&[Stream {
                capability: s.capability,
                version: s.version,
                payload: s.payload,
            }])
            .unwrap();
            // stored as it is, whatever codec the block had
            (codec_id, uncompressed_length) = (0, stored.len() as u32);
        }
        blocks.push(RawBlock {
            kind: e.kind,
            subtype: e.subtype,
            cell: e.cell,
            count: e.count,
            bounds: e.bounds,
            flags: e.flags & (FLAG_MUST_UNDERSTAND | FLAG_HAS_BOUNDS),
            codec: codec_id,
            capability: e.capability,
            uncompressed_length,
            stored,
        });
    }
    assemble(r.manifest().clone(), &blocks, Integrity::Full).expect("the file is laid out again")
}
