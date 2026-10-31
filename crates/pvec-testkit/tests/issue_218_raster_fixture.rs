//! Issue #218 (R1.4): the test-only tiler cuts a synthetic image into the tiles of a pyramid and encodes each
//! as a valid PNG with stored deflate blocks, so no image codec enters any crate; and the raster fixture built
//! from it has a digest, with an evidence class that says what it proves.
//!
//! The claim is the container's: the tile bytes written are the tile bytes read, and the six coefficients come
//! back as the same bits. It is not a harness truth. `KNOWN_TRUTH_EXACT` supports source intent (what a
//! drawing meant), and keeping bytes is not one of the corpus claims, so this fixture is not in the corpus and
//! `pvec-gold-fixtures` is not touched.

use pvec_capabilities::Version;
use pvec_components::{
    Band, PixelFormat, Placement, RASTER_CAPABILITY, RasterId, Stream, TileCodec, encode_rasters,
};
use pvec_container::Bounds;
use pvec_container::blocks::raster::TileIn;
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_testkit::tiler::{Shape, pixel, png, tiles};
use std::path::PathBuf;

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in bytes {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// A reader of exactly the PNGs the tiler makes, written separately from the tiler: signature, chunks with their
/// CRCs, the zlib stream with stored blocks and its Adler sum. Returns (width, height, depth, colour, scanlines).
fn read_png(b: &[u8]) -> (u32, u32, u8, u8, Vec<u8>) {
    assert_eq!(&b[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut at = 8;
    let (mut ihdr, mut idat, mut ended) = (None, Vec::new(), false);
    while at < b.len() {
        assert!(!ended, "nothing follows IEND");
        let len = be32(b, at) as usize;
        let kind = &b[at + 4..at + 8];
        let data = &b[at + 8..at + 8 + len];
        assert_eq!(be32(b, at + 8 + len), crc32(&b[at + 4..at + 8 + len]), "CRC of {kind:?}");
        match kind {
            b"IHDR" => ihdr = Some(data.to_vec()),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => ended = true,
            other => panic!("unexpected chunk {other:?}"),
        }
        at += 12 + len;
    }
    assert!(ended);
    let h = ihdr.expect("IHDR");
    assert_eq!(&h[10..13], &[0, 0, 0], "deflate, adaptive filtering, no interlace");
    // zlib: 0x78 0x01, stored blocks, Adler-32
    assert_eq!(&idat[..2], &[0x78, 0x01]);
    let mut z = 2;
    let mut raw = Vec::new();
    loop {
        let last = idat[z] & 1 == 1;
        assert_eq!(idat[z] >> 1, 0, "stored block");
        let len = usize::from(u16::from_le_bytes([idat[z + 1], idat[z + 2]]));
        assert_eq!(u16::from_le_bytes([idat[z + 3], idat[z + 4]]), !(len as u16));
        raw.extend_from_slice(&idat[z + 5..z + 5 + len]);
        z += 5 + len;
        if last {
            break;
        }
    }
    let (mut a, mut s) = (1u32, 0u32);
    for x in &raw {
        a = (a + u32::from(*x)) % 65521;
        s = (s + a) % 65521;
    }
    assert_eq!(be32(&idat, z), (s << 16) | a, "Adler-32");
    assert_eq!(z + 4, idat.len());
    (be32(&h, 0), be32(&h, 4), h[8], h[9], raw)
}

#[test]
fn a_tile_is_a_valid_png_whose_pixels_are_the_ones_it_was_cut_from() {
    for (channels, depth) in [(1u8, 8u8), (2, 8), (3, 8), (4, 8), (1, 1)] {
        let edge = 256;
        let bytes = png(edge, channels, depth, |x, y| pixel(x + 3, y + 5, channels, depth));
        let (w, h, d, colour, raw) = read_png(&bytes);
        assert_eq!((w, h, d), (edge, edge, depth));
        assert_eq!(colour, [0, 4, 2, 6][usize::from(channels) - 1]);
        let row = if depth == 1 { 32 } else { edge as usize * usize::from(channels) };
        assert_eq!(raw.len(), (row + 1) * edge as usize);
        for y in [0u32, 1, 100, 255] {
            let line = &raw[(row + 1) * y as usize..(row + 1) * (y as usize + 1)];
            assert_eq!(line[0], 0, "filter none");
            for x in [0u32, 1, 7, 8, 200, 255] {
                let p = pixel(x + 3, y + 5, channels, depth);
                if depth == 1 {
                    assert_eq!((line[1 + x as usize / 8] >> (7 - x % 8)) & 1, p[0], "({x}, {y})");
                } else {
                    let at = 1 + x as usize * usize::from(channels);
                    assert_eq!(
                        &line[at..at + usize::from(channels)],
                        &p[..usize::from(channels)],
                        "({x}, {y})"
                    );
                }
            }
        }
    }
}

#[test]
fn the_pyramid_is_the_one_the_placement_fixes() {
    let shape = Shape { width: 1500, height: 900, tile_edge: 256, channels: 1, bit_depth: 8 };
    assert_eq!(shape.levels(), 4);
    assert_eq!(shape.grid(), vec![(1, 1), (2, 1), (3, 2), (6, 4)]);
    let all = tiles(shape, |_, _, _| true);
    assert_eq!(all.len(), 1 + 2 + 6 + 24);
    let sparse = tiles(shape, |l, c, r| (l + c + r) % 2 == 0);
    assert!(sparse.iter().any(|t| t.bytes.is_empty()) && sparse.iter().any(|t| !t.bytes.is_empty()));
    // the same call gives the same bytes
    assert_eq!(all, tiles(shape, |_, _, _| true));
    // a one-tile image is one level
    assert_eq!(Shape { width: 200, height: 100, ..shape }.levels(), 1);
}

/// The fixture: a drawing with one line and one sparse gray raster of 700 by 500 in tiles of 256, upright at one
/// unit a pixel, its tiles the tiler's PNGs.
fn fixture() -> (Vec<u8>, Vec<TileIn>, Placement) {
    let shape = Shape { width: 700, height: 500, tile_edge: 256, channels: 1, bit_depth: 8 };
    let placement = Placement {
        pixel_to_source: [1.0, 0.0, 0.0, -1.0, 0.0, 500.0],
        width: 700,
        height: 500,
        visible_box: [0, 0, 700, 500],
        tile_edge: 256,
        levels: shape.levels() as u8,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    };
    let given = tiles(shape, |l, c, r| !(l == 2 && c == 2 && r == 1));
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-test-writer".into(),
            producer_version: "0.0.0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    let bounds = Bounds { min: [0.0, 0.0], max: [700.0, 500.0] };
    w.push_line(
        EntityMeta { id: EntityId::new(0).unwrap(), style: 0, cell: 0, bounds },
        [0.0, 0.0],
        [700.0, 500.0],
    )
    .unwrap();
    w.push_stream(Stream {
        capability: RASTER_CAPABILITY,
        version: Version::new(1, 1, 0),
        payload: encode_rasters(std::slice::from_ref(&placement)).unwrap(),
    })
    .unwrap();
    w.push_raster_tiles(RasterId::new(0).unwrap(), given.clone()).unwrap();
    (w.finish().unwrap(), given, placement)
}

fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/raster-digests.txt")
}

#[test]
fn the_fixture_keeps_every_byte_it_was_given_and_the_placement_as_the_same_bits() {
    let (bytes, given, placement) = fixture();
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    reader.validate().unwrap();
    reader.validate_raster(0).unwrap();
    for t in &given {
        let got = reader.read_raster_tile(0, t.level, t.col, t.row).unwrap();
        if t.bytes.is_empty() {
            assert!(got.is_none());
        } else {
            assert_eq!(got.unwrap().as_slice(), &t.bytes[..]);
            // and each one is a PNG a decoder reads
            let (w, h, ..) = read_png(&t.bytes);
            assert_eq!((w, h), (256, 256));
        }
    }
    let index = reader.raster_index(0).unwrap();
    index.check_grid(placement.width, placement.height, u32::from(placement.tile_edge)).unwrap();
}

#[test]
fn the_fixture_has_a_digest_in_the_golden_file_and_an_evidence_class_that_says_what_it_proves() {
    let (bytes, ..) = fixture();
    let digest = pvec_corpus::sha256::hex_digest(&bytes);
    let line = format!("raster-synthetic-gray-700x500 {digest} container-claim-byte-preservation\n");
    if std::env::var_os("PVEC_BLESS_RASTER_FIXTURE").is_some() {
        std::fs::write(golden(), &line).unwrap();
    }
    let committed = std::fs::read_to_string(golden()).unwrap_or_default();
    assert_eq!(committed, line, "tests/golden/raster-digests.txt (PVEC_BLESS_RASTER_FIXTURE=1 writes it)");
    // the class is a container claim: it is not one of the corpus's evidence classes, and says so
    for corpus_class in ["KNOWN_TRUTH_EXACT", "DUAL_ORACLE_EQUIVALENT", "REAL_WORLD_DIFFERENTIAL"] {
        assert!(!committed.contains(corpus_class));
    }
    let doc = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/RASTER.md"))
        .unwrap();
    let flat = doc.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("container-claim-byte-preservation") && flat.contains("not a KNOWN_TRUTH_EXACT truth"),
        "docs/RASTER.md does not say what the raster fixture proves"
    );
}
