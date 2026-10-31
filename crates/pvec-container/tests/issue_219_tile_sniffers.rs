//! Issue #219 (R1.5): the three bounded sniffers. PVec never decodes a pixel, so the only thing between a hostile
//! tile and the host's decoder is a header check: the tile says it is `tile_edge` by `tile_edge` in the placement's
//! bit depth and channels, it is exactly one image and nothing after it, and it is no larger than an honest
//! encoding could be. Every hostile case is refused by name, and the sniffers never look at more than a constant
//! number of bytes of a tile, whatever its length.
#![allow(missing_docs)]

#[path = "../../pvec-testkit/src/tilegen.rs"]
mod tilegen;

use pvec_components::{Band, PixelFormat, Placement, TileCodec};
use pvec_container::blocks::sniff::{
    JPEG, JPEG_WALK, MAX_EXAMINED, MAX_PNG_CHUNKS, PNG, TileRefusal, WEBP, jpeg, max_tile_bytes, png,
    sniff_codec, sniff_tile, webp,
};

const ALLOWANCE: u32 = 65_536;

fn placement(codec: TileCodec, format: PixelFormat, depth: u8, edge: u16) -> Placement {
    Placement {
        pixel_to_source: [1.0, 0.0, 0.0, -1.0, 0.0, 1000.0],
        width: 1000,
        height: 1000,
        visible_box: [0, 0, 1000, 1000],
        tile_edge: edge,
        levels: 3,
        tile_codec: codec,
        pixel_format: format,
        bit_depth: depth,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }
}

fn refused(codec: u8, edge: u32, channels: u8, depth: u8, bytes: &[u8]) -> TileRefusal {
    sniff_codec(codec, edge, channels, depth, ALLOWANCE, bytes).expect_err("the tile must be refused")
}

/// The variants libviprs writes, each as a tile of `edge`: (name, codec, channels, depth, bytes).
fn accepted(edge: u32) -> Vec<(&'static str, u8, u8, u8, Vec<u8>)> {
    let d = tilegen::payload(700, 3);
    vec![
        ("png gray 1-bit", PNG, 1, 1, tilegen::png(edge, 1, 1, &d)),
        ("png gray 8-bit", PNG, 1, 8, tilegen::png(edge, 1, 8, &d)),
        ("png gray-alpha", PNG, 2, 8, tilegen::png(edge, 2, 8, &d)),
        ("png rgb", PNG, 3, 8, tilegen::png(edge, 3, 8, &d)),
        ("png rgba", PNG, 4, 8, tilegen::png(edge, 4, 8, &d)),
        ("png rgba, many IDATs", PNG, 4, 8, tilegen::png_with(edge, edge, 8, 6, 0, &d, 40)),
        ("jpeg gray baseline", JPEG, 1, 8, tilegen::jpeg(edge, 1, false, &d)),
        ("jpeg rgb baseline", JPEG, 3, 8, tilegen::jpeg(edge, 3, false, &d)),
        ("jpeg gray progressive", JPEG, 1, 8, tilegen::jpeg(edge, 1, true, &d)),
        ("jpeg rgb progressive", JPEG, 3, 8, tilegen::jpeg(edge, 3, true, &d)),
        ("webp VP8", WEBP, 3, 8, tilegen::webp_vp8(edge as u16, edge as u16, &d)),
        ("webp VP8L", WEBP, 4, 8, tilegen::webp_vp8l(edge, edge, &d)),
        ("webp VP8X lossless", WEBP, 4, 8, tilegen::webp_vp8x(edge, edge, false, true, &d)),
        ("webp VP8X lossy", WEBP, 3, 8, tilegen::webp_vp8x(edge, edge, false, false, &d)),
    ]
}

#[test]
fn every_variant_libviprs_writes_passes_at_every_edge() {
    for edge in [256u32, 512, 1024] {
        for (name, codec, channels, depth, bytes) in accepted(edge) {
            let n = sniff_codec(codec, edge, channels, depth, ALLOWANCE, &bytes)
                .unwrap_or_else(|e| panic!("{name} at {edge}: {e}"));
            assert!(n > 0 && n <= MAX_EXAMINED, "{name}: {n} bytes examined");
        }
    }
}

#[test]
fn a_tile_is_checked_against_the_placement_it_belongs_to() {
    let p = placement(TileCodec::Png, PixelFormat::Rgb, 8, 256);
    sniff_tile(&p, ALLOWANCE, &tilegen::png(256, 3, 8, &[1, 2, 3])).expect("rgb png under an rgb placement");
    for (bytes, why) in [
        (tilegen::png(256, 4, 8, &[1]), "rgba under rgb"),
        (tilegen::png(256, 1, 8, &[1]), "gray under rgb"),
        (tilegen::png(512, 3, 8, &[1]), "512 under 256"),
        (tilegen::jpeg(256, 3, false, &[1]), "a jpeg under a png placement"),
    ] {
        assert!(sniff_tile(&p, ALLOWANCE, &bytes).is_err(), "{why}");
    }
    let p = placement(TileCodec::Webp, PixelFormat::Rgba, 8, 512);
    sniff_tile(&p, ALLOWANCE, &tilegen::webp_vp8l(512, 512, &[1])).unwrap();
}

#[test]
fn the_rules_are_named() {
    let hostile = tilegen::png(256, 1, 8, &[1]);
    assert_eq!(refused(PNG, 512, 1, 8, &hostile).rule(), "RASTER-03");
    assert_eq!(refused(0, 256, 1, 8, &hostile).rule(), "RASTER-04");
    let big = vec![0u8; max_tile_bytes(256, ALLOWANCE) as usize + 1];
    assert_eq!(refused(PNG, 256, 1, 8, &big).rule(), "LIMIT-15");
}

// LIMIT-15

#[test]
fn limit_15_the_ceiling_is_twice_the_bytes_of_an_rgba_tile_plus_the_allowance() {
    assert_eq!(max_tile_bytes(256, ALLOWANCE), 589_824);
    assert_eq!(max_tile_bytes(512, ALLOWANCE), 2_162_688);
    assert_eq!(max_tile_bytes(1024, ALLOWANCE), 8_454_144);
}

#[test]
fn limit_15_a_tile_at_the_ceiling_passes_and_one_byte_more_is_refused_without_reading_it() {
    let ceiling = max_tile_bytes(256, ALLOWANCE) as usize;
    // a PNG of exactly the ceiling: the filler is the payload of its one IDAT
    let header = tilegen::png(256, 1, 8, &[]).len();
    let at = tilegen::png(256, 1, 8, &tilegen::payload(ceiling - header, 9));
    assert_eq!(at.len(), ceiling);
    assert!(sniff_codec(PNG, 256, 1, 8, ALLOWANCE, &at).is_ok());
    let over = tilegen::png(256, 1, 8, &tilegen::payload(ceiling - header + 1, 9));
    assert_eq!(over.len(), ceiling + 1);
    let e = refused(PNG, 256, 1, 8, &over);
    assert_eq!(e, TileRefusal::TooLarge { found: ceiling as u64 + 1, ceiling: ceiling as u64 });
    // refused on the length alone: bytes that are no image at all give the same answer
    assert_eq!(refused(PNG, 256, 1, 8, &vec![0u8; ceiling + 1]), e);
}

// RASTER-03: the hostile cases, each by name

#[test]
fn raster_03_a_valid_png_followed_by_an_archive_or_a_page_is_refused() {
    let png = tilegen::png(256, 1, 8, &tilegen::payload(500, 1));
    let zip = b"PK\x03\x04\x14\x00\x00\x00\x08\x00 an archive with a name and some deflated bytes";
    let html = b"<!doctype html><script>alert(document.domain)</script>";
    for (name, tail) in [("a zip", &zip[..]), ("a page", &html[..]), ("a single byte", &[0u8][..])] {
        let mut polyglot = png.clone();
        polyglot.extend_from_slice(tail);
        assert_eq!(refused(PNG, 256, 1, 8, &polyglot), TileRefusal::Trailing, "{name}");
    }
    // the same with an IEND of its own at the very end, which a check of the last twelve bytes alone would accept
    let mut disguised = png.clone();
    disguised.extend_from_slice(zip);
    disguised.extend_from_slice(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
    assert_eq!(refused(PNG, 256, 1, 8, &disguised), TileRefusal::Trailing);
}

#[test]
fn raster_03_a_jpeg_or_a_webp_followed_by_a_payload_is_refused() {
    let mut jpg = tilegen::jpeg(256, 3, false, &tilegen::payload(300, 2));
    jpg.extend_from_slice(b"PK\x03\x04 an archive");
    assert_eq!(refused(JPEG, 256, 3, 8, &jpg), TileRefusal::Trailing);
    let mut web = tilegen::webp_vp8l(256, 256, &tilegen::payload(300, 2));
    web.extend_from_slice(b"<html>");
    assert_eq!(refused(WEBP, 256, 4, 8, &web), TileRefusal::Trailing);
    // a RIFF size that is short of the tile, and one that is past it
    for delta in [-2i64, 2] {
        let mut web = tilegen::webp_vp8l(256, 256, &tilegen::payload(300, 2));
        let size = (i64::from(u32::from_le_bytes(web[4..8].try_into().unwrap())) + delta) as u32;
        web[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(refused(WEBP, 256, 4, 8, &web), TileRefusal::Trailing, "RIFF size {delta:+}");
    }
}

#[test]
fn raster_03_an_animated_webp_is_refused() {
    let animated = tilegen::webp_vp8x(256, 256, true, true, &tilegen::payload(100, 1));
    assert_eq!(refused(WEBP, 256, 4, 8, &animated), TileRefusal::Animated);
}

#[test]
fn raster_03_an_interlaced_png_is_refused() {
    let interlaced = tilegen::png_with(256, 256, 8, 0, 1, &tilegen::payload(100, 1), 1);
    assert_eq!(refused(PNG, 256, 1, 8, &interlaced), TileRefusal::Interlace);
}

#[test]
fn raster_03_a_sixteen_bit_png_and_a_palette_png_are_refused() {
    let sixteen = tilegen::png_with(256, 256, 16, 0, 0, &tilegen::payload(100, 1), 1);
    assert_eq!(refused(PNG, 256, 1, 8, &sixteen), TileRefusal::Format);
    let palette = tilegen::png_with(256, 256, 8, 3, 0, &tilegen::payload(100, 1), 1);
    for channels in 1..=4 {
        assert_eq!(refused(PNG, 256, channels, 8, &palette), TileRefusal::Format, "{channels} channels");
    }
    // a two- or four-bit PNG is a legal PNG and not a tile
    for depth in [2u8, 4] {
        let d = tilegen::png_with(256, 256, depth, 0, 0, &[1], 1);
        assert_eq!(refused(PNG, 256, 1, 8, &d), TileRefusal::Format);
    }
}

#[test]
fn raster_03_a_jpeg_with_four_components_or_two_is_refused() {
    for nf in [2u8, 4] {
        let j = tilegen::jpeg(256, nf, false, &tilegen::payload(100, 1));
        for channels in [1u8, 3] {
            assert_eq!(
                refused(JPEG, 256, channels, 8, &j),
                TileRefusal::Format,
                "Nf {nf} for {channels} channels"
            );
        }
    }
    // Nf is the placement's: a gray jpeg under an rgb placement and the other way round
    assert_eq!(refused(JPEG, 256, 3, 8, &tilegen::jpeg(256, 1, false, &[1])), TileRefusal::Format);
    assert_eq!(refused(JPEG, 256, 1, 8, &tilegen::jpeg(256, 3, false, &[1])), TileRefusal::Format);
}

#[test]
fn raster_03_a_jpeg_whose_frame_header_is_past_64_kib_is_refused_without_walking_further() {
    // padding in comment segments puts the SOF past the walk; what follows is poison that must not be reached
    let late = tilegen::jpeg_with(256, 256, 3, false, 70_000, &tilegen::payload(100, 1), true);
    let sof = late.windows(2).position(|w| w == [0xFF, 0xC0]).expect("a SOF0");
    assert!(sof > JPEG_WALK, "the frame header is at {sof}");
    assert_eq!(refused(JPEG, 256, 3, 8, &late), TileRefusal::Walk);
    // the same tile with the walk's limit moved to where its frame header is would pass, so the refusal is the walk
    let early = tilegen::jpeg_with(256, 256, 3, false, 30_000, &tilegen::payload(100, 1), true);
    assert!(early.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap() < JPEG_WALK);
    assert!(sniff_codec(JPEG, 256, 3, 8, ALLOWANCE, &early).is_ok());
}

#[test]
fn raster_03_a_header_that_lies_about_the_size_is_refused() {
    for (w, h) in [(255, 256), (256, 255), (257, 256), (512, 512), (256, 512), (0, 0), (65_535, 256)] {
        let p = tilegen::png_with(w, h, 8, 0, 0, &[1, 2, 3], 1);
        assert_eq!(refused(PNG, 256, 1, 8, &p), TileRefusal::Size { width: w, height: h }, "png {w} by {h}");
    }
    for (w, h) in [(255u32, 256u32), (256, 257), (512, 512)] {
        let j = tilegen::jpeg_with(w, h, 3, false, 0, &[1], true);
        assert_eq!(
            refused(JPEG, 256, 3, 8, &j),
            TileRefusal::Size { width: w, height: h },
            "jpeg {w} by {h}"
        );
        let l = tilegen::webp_vp8l(w, h, &[1]);
        assert_eq!(
            refused(WEBP, 256, 4, 8, &l),
            TileRefusal::Size { width: w, height: h },
            "vp8l {w} by {h}"
        );
        let x = tilegen::webp_vp8x(w, h, false, true, &[1]);
        assert_eq!(
            refused(WEBP, 256, 4, 8, &x),
            TileRefusal::Size { width: w, height: h },
            "vp8x {w} by {h}"
        );
        let v = tilegen::webp_vp8(w as u16, h as u16, &[1]);
        assert_eq!(refused(WEBP, 256, 3, 8, &v), TileRefusal::Size { width: w, height: h }, "vp8 {w} by {h}");
    }
}

#[test]
fn raster_03_a_bit_depth_the_placement_does_not_have_is_refused() {
    let eight = tilegen::png(256, 1, 8, &[1]);
    let one = tilegen::png(256, 1, 1, &[1]);
    assert_eq!(refused(PNG, 256, 1, 1, &eight), TileRefusal::Format);
    assert_eq!(refused(PNG, 256, 1, 8, &one), TileRefusal::Format);
    // one bit exists for gray only: an rgb PNG that says it is one bit deep is no PNG
    let rgb_one = tilegen::png_with(256, 256, 1, 2, 0, &[1], 1);
    assert_eq!(refused(PNG, 256, 3, 1, &rgb_one), TileRefusal::Format);
}

#[test]
fn raster_03_the_edge_of_the_structure_is_a_refusal_and_never_a_panic() {
    for (name, codec, channels, depth, bytes) in accepted(256) {
        assert!(sniff_codec(codec, 256, channels, depth, ALLOWANCE, &bytes).is_ok());
        // every prefix of an accepted tile is refused, and none panics
        for n in (0..bytes.len()).step_by(1) {
            assert!(
                sniff_codec(codec, 256, channels, depth, ALLOWANCE, &bytes[..n]).is_err(),
                "{name}: a prefix of {n} of {} bytes passed",
                bytes.len()
            );
        }
        // and so is each tile with one byte of its header changed to every other value it can take
        for at in 0..bytes.len().min(40) {
            for v in [0u8, 1, 0x7F, 0x80, 0xFF] {
                let mut b = bytes.clone();
                b[at] = v;
                let _ = sniff_codec(codec, 256, channels, depth, ALLOWANCE, &b);
            }
        }
    }
    // nothing at all
    for codec in [PNG, JPEG, WEBP] {
        assert_eq!(refused(codec, 256, 1, 8, &[]), TileRefusal::Truncated);
    }
}

// RASTER-04

#[test]
fn raster_04_a_codec_that_is_none_of_the_three_and_a_raw_tile_are_refused_by_pair() {
    let raw = vec![0u8; 256 * 256];
    for codec in [0u8, 4, 5, 255] {
        assert_eq!(refused(codec, 256, 1, 8, &raw), TileRefusal::UnknownCodec { id: codec });
    }
    let p = placement(TileCodec::Unknown(4), PixelFormat::Gray, 8, 256);
    assert_eq!(sniff_tile(&p, ALLOWANCE, &raw), Err(TileRefusal::UnknownCodec { id: 4 }));
}

#[test]
fn raster_04_a_jpeg_under_a_placement_with_alpha_or_one_bit_is_refused_before_a_byte_is_read() {
    let j = tilegen::jpeg(256, 3, false, &[1]);
    for (channels, depth) in [(4u8, 8u8), (2, 8), (1, 1)] {
        assert_eq!(
            refused(JPEG, 256, channels, depth, &j),
            TileRefusal::Pair,
            "{channels} channels at {depth}"
        );
    }
    let rgba = placement(TileCodec::Jpeg, PixelFormat::Rgba, 8, 256);
    assert_eq!(sniff_tile(&rgba, ALLOWANCE, &j), Err(TileRefusal::Pair));
}

// the bound on the work

#[test]
fn a_tile_costs_a_constant_number_of_bytes_looked_at_whatever_its_length() {
    // a tile at the ceiling for 1024 is 8 MB; the sniffer looks at the same few bytes of it as of a small one
    let small = tilegen::png(1024, 1, 8, &tilegen::payload(100, 1));
    let big = tilegen::png(1024, 1, 8, &tilegen::payload(8_000_000, 1));
    let (a, b) = (
        sniff_codec(PNG, 1024, 1, 8, ALLOWANCE, &small).unwrap(),
        sniff_codec(PNG, 1024, 1, 8, ALLOWANCE, &big).unwrap(),
    );
    assert_eq!(a, b, "the length of the IDAT is not looked at");
    assert!(b <= 64, "{b}");
    let j = tilegen::jpeg_with(1024, 1024, 3, true, 50_000, &tilegen::payload(5_000_000, 1), true);
    let n = sniff_codec(JPEG, 1024, 3, 8, ALLOWANCE, &j).unwrap();
    assert!(n <= MAX_EXAMINED, "{n}");
    // a PNG framed in as many chunks as the walk allows is examined in 8 bytes a chunk
    let many = tilegen::png_with(256, 256, 8, 0, 0, &vec![7u8; MAX_PNG_CHUNKS - 4], MAX_PNG_CHUNKS - 4);
    let n = sniff_codec(PNG, 256, 1, 8, ALLOWANCE, &many).unwrap();
    assert!(n <= 33 + 8 * MAX_PNG_CHUNKS && n <= MAX_EXAMINED, "{n}");
    // and one framed in more is refused instead of walked
    let too_many =
        tilegen::png_with(256, 256, 8, 0, 0, &vec![7u8; MAX_PNG_CHUNKS + 100], MAX_PNG_CHUNKS + 100);
    assert_eq!(refused(PNG, 256, 1, 8, &too_many), TileRefusal::Malformed);
    // the three entry points agree with the dispatcher
    assert!(png(&small, 1024, 1, 8).is_ok() && jpeg(&j, 1024, 3).is_ok());
    assert!(webp(&tilegen::webp_vp8l(1024, 1024, &[1]), 1024).is_ok());
}
