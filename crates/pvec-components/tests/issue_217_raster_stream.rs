//! The raster placement stream (issue 217, ADR 0018, `docs/RASTER.md` section 1): one row per `RasterId` with the
//! affine into source space, the grid, the codec and format tags, opacity, band, an optional layer and the
//! provenance strings. Every rule below refuses the raster and never the file; LIMIT-13 bounds the count.
#![allow(missing_docs)]

use pvec_capabilities::{CapabilityId, Reader, Registry, Version};
use pvec_components::*;

/// The test vector of `docs/RASTER.md` section 5: a rotated, sheared 1024 by 512 image.
fn vector() -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
        width: 1024,
        height: 512,
        visible_box: [0, 0, 1024, 512],
        tile_edge: 512,
        levels: 2,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Rgba,
        bit_depth: 8,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }
}

fn with(f: impl FnOnce(&mut Placement)) -> Placement {
    let mut p = vector();
    f(&mut p);
    p
}

fn rule_of(p: Placement) -> &'static str {
    encode_rasters(&[p]).expect_err("refused").rule()
}

fn bits(p: &Placement) -> Vec<u64> {
    p.pixel_to_source.iter().map(|x| x.to_bits()).collect()
}

// ---------------------------------------------------------------- round trip

#[test]
fn a_placement_round_trips_with_the_six_coefficients_bit_equal() {
    let full = Placement {
        layer_index: Some(3),
        source_digest: Some([0xAB; 32]),
        attribution: "Survey of the north wing, 2026".into(),
        opacity: 128,
        band: Band::Overlay,
        ..vector()
    };
    let bytes = encode_rasters(&[vector(), full.clone()]).unwrap();
    let back = decode_rasters(&bytes).unwrap();
    assert_eq!(back, vec![vector(), full]);
    assert_eq!(bits(&back[0]), bits(&vector()));
    // the record is 88 bytes and then its tail, after the u32 count
    assert_eq!(bytes.len(), 4 + 88 + 88 + 32 + "Survey of the north wing, 2026".len());
}

#[test]
fn negative_zero_is_written_as_positive_zero_before_the_bits_are_compared() {
    let p = with(|p| p.pixel_to_source = [0.5, -0.0, -0.0, -0.5, -0.0, 50.0]);
    let back = decode_rasters(&encode_rasters(&[p]).unwrap()).unwrap();
    assert_eq!(back[0].pixel_to_source, [0.5, 0.0, 0.0, -0.5, 0.0, 50.0]);
    assert_eq!(back[0].pixel_to_source.map(f64::to_bits), [0.5, 0.0, 0.0, -0.5, 0.0, 50.0].map(f64::to_bits));
}

#[test]
fn an_empty_stream_is_a_count_of_zero_and_nothing_else() {
    assert_eq!(encode_rasters(&[]).unwrap(), 0u32.to_le_bytes());
    assert_eq!(decode_rasters(&0u32.to_le_bytes()).unwrap(), vec![]);
}

// ---------------------------------------------------------------- NP-19 by name, and which rule is in force

#[test]
fn a_reflection_and_a_non_uniform_scale_are_legal_and_d_below_zero_is_the_normal_case() {
    for m in [
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],   // mirrored in y against the upright image
        [-1.0, 0.0, 0.0, -1.0, 5.0, 5.0], // a rotation by half a turn
        [2.0, 0.0, 0.0, -0.25, 0.0, 0.0], // non-uniform
        [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
    ] {
        assert!(encode_rasters(&[with(|p| p.pixel_to_source = m)]).is_ok(), "{m:?}");
    }
}

#[test]
fn a_map_that_is_singular_under_pvecs_rule_is_refused_though_the_1e_12_rule_would_take_it() {
    // det = 1e-11: above libviprs's 1e-12, at or below PVec's `REL * big^2` = 1e-10 with big = 1
    let m: [f64; 6] = [1.0, 0.0, 0.0, 1e-11, 0.0, 0.0];
    let det = m[0] * m[3] - m[1] * m[2];
    assert!(det.abs() > 1e-12, "the 1e-12 rule accepts this map");
    assert_eq!(rule_of(with(|p| p.pixel_to_source = m)), "RASTER-05");
    // an exactly singular map too
    assert_eq!(rule_of(with(|p| p.pixel_to_source = [1.0, 2.0, 2.0, 4.0, 0.0, 0.0])), "RASTER-05");
}

#[test]
fn raster_05_refuses_the_2_pow_20_bound_non_finite_and_a_footprint_corner_outside_np_02() {
    let bound = f64::from(1u32 << 20);
    assert!(encode_rasters(&[with(|p| p.pixel_to_source = [bound, 0.0, 0.0, -1.0, 0.0, 0.0])]).is_ok());
    assert_eq!(rule_of(with(|p| p.pixel_to_source = [bound * 2.0, 0.0, 0.0, -1.0, 0.0, 0.0])), "RASTER-05");
    assert_eq!(rule_of(with(|p| p.pixel_to_source[4] = f64::NAN)), "RASTER-05");
    assert_eq!(rule_of(with(|p| p.pixel_to_source[5] = f64::INFINITY)), "RASTER-05");
    // 2^20 * 6e7 pixels is past NP-02's 2^40, though every coefficient is legal and the grid fits
    let huge = with(|p| {
        p.pixel_to_source = [bound, 0.0, 0.0, -1.0, 0.0, 0.0];
        p.width = 60_000_000;
        p.visible_box = [0, 0, p.width, p.height];
        p.tile_edge = 1024;
        p.levels = 17;
    });
    assert_eq!(rule_of(huge), "RASTER-05");
    // a translation at the NP-02 bound is legal, one past it is not
    let edge = 2f64.powi(40);
    assert!(
        encode_rasters(&[with(|p| p.pixel_to_source = [0.5, 0.0, 0.0, -0.5, edge - 1024.0, 0.0])]).is_ok()
    );
    assert_eq!(rule_of(with(|p| p.pixel_to_source = [0.5, 0.0, 0.0, -0.5, edge * 2.0, 0.0])), "RASTER-05");
}

// ---------------------------------------------------------------- RASTER-01, the grid

#[test]
fn raster_01_refuses_a_tile_edge_outside_256_512_1024_levels_that_do_not_match_and_a_box_outside_the_image() {
    for edge in [0u16, 128, 255, 257, 2048] {
        assert_eq!(rule_of(with(|p| p.tile_edge = edge)), "RASTER-01", "edge {edge}");
    }
    // 1024 x 512 at an edge of 512: two tiles across, so exactly two levels
    for levels in [0u8, 1, 3, 255] {
        assert_eq!(rule_of(with(|p| p.levels = levels)), "RASTER-01", "levels {levels}");
    }
    assert!(encode_rasters(&[with(|p| p.levels = 2)]).is_ok());
    // a pyramid that stops at the first one-tile level: 512 x 512 at an edge of 512 is one level
    let one = with(|p| {
        p.width = 512;
        p.visible_box = [0, 0, 512, 512];
        p.levels = 1;
    });
    assert!(encode_rasters(&[one.clone()]).is_ok());
    assert_eq!(rule_of(Placement { levels: 2, ..one }), "RASTER-01");
    for visible in [[0, 0, 1025, 512], [0, 0, 1024, 513], [10, 10, 10, 20], [20, 0, 10, 512], [0, 0, 0, 0]] {
        assert_eq!(rule_of(with(|p| p.visible_box = visible)), "RASTER-01", "{visible:?}");
    }
    assert_eq!(rule_of(with(|p| p.width = 0)), "RASTER-01");
    assert_eq!(rule_of(with(|p| p.height = 0)), "RASTER-01");
}

#[test]
fn raster_01_keeps_cols_and_rows_inside_the_u16_of_the_index() {
    // 65,536 columns of 256 px
    let wide = with(|p| {
        p.pixel_to_source = [1e-3, 0.0, 0.0, -1e-3, 0.0, 0.0];
        p.tile_edge = 256;
        p.width = 65_536 * 256;
        p.height = 256;
        p.visible_box = [0, 0, p.width, 256];
        p.levels = 17;
    });
    assert_eq!(rule_of(wide), "RASTER-01");
}

// ---------------------------------------------------------------- RASTER-04, codec and format

#[test]
fn raster_04_allows_jpeg_only_with_gray_or_rgb_at_eight_bits() {
    let jpeg = |format, depth| {
        with(move |p| {
            p.tile_codec = TileCodec::Jpeg;
            p.pixel_format = format;
            p.bit_depth = depth;
        })
    };
    assert!(encode_rasters(&[jpeg(PixelFormat::Gray, 8)]).is_ok());
    assert!(encode_rasters(&[jpeg(PixelFormat::Rgb, 8)]).is_ok());
    assert_eq!(rule_of(jpeg(PixelFormat::GrayAlpha, 8)), "RASTER-04");
    assert_eq!(rule_of(jpeg(PixelFormat::Rgba, 8)), "RASTER-04");
    assert_eq!(rule_of(jpeg(PixelFormat::Gray, 1)), "RASTER-04");
    // PNG and WebP carry alpha, and a bilevel PNG is legal
    for codec in [TileCodec::Png, TileCodec::Webp] {
        assert!(encode_rasters(&[with(|p| p.tile_codec = codec)]).is_ok());
    }
    assert!(
        encode_rasters(&[with(|p| {
            p.pixel_format = PixelFormat::Gray;
            p.bit_depth = 1;
        })])
        .is_ok()
    );
}

#[test]
fn an_unknown_codec_id_is_a_raster_level_skip_with_its_own_reason_and_not_an_error() {
    let p = with(|p| p.tile_codec = TileCodec::Unknown(9));
    let back = decode_rasters(&encode_rasters(&[vector(), p.clone()]).unwrap()).expect("the stream opens");
    assert_eq!(back.len(), 2, "the raster keeps its id, which is its position");
    assert_eq!(back[0].skip(), None);
    assert_eq!(back[1].skip(), Some(RasterSkip::UnknownCodec(9)));
    assert_eq!(back[1], p);
    // 0 is not a codec either
    let zero = with(|p| p.tile_codec = TileCodec::Unknown(0));
    assert_eq!(
        decode_rasters(&encode_rasters(&[zero]).unwrap()).unwrap()[0].skip(),
        Some(RasterSkip::UnknownCodec(0))
    );
}

#[test]
fn the_wire_tags_map_to_their_words_and_back() {
    for (codec, word) in [(TileCodec::Png, 1u8), (TileCodec::Jpeg, 2), (TileCodec::Webp, 3)] {
        assert_eq!((codec.word(), TileCodec::from_word(word)), (word, codec));
    }
    assert_eq!(TileCodec::from_word(0), TileCodec::Unknown(0));
    assert_eq!(TileCodec::from_word(200), TileCodec::Unknown(200));
    for (format, word) in
        [(PixelFormat::Gray, 0u8), (PixelFormat::GrayAlpha, 1), (PixelFormat::Rgb, 2), (PixelFormat::Rgba, 3)]
    {
        assert_eq!((format.word(), PixelFormat::from_word(word)), (word, Some(format)));
    }
    assert_eq!(PixelFormat::from_word(4), None);
    for (band, word) in [(Band::Underlay, 0u8), (Band::Overlay, 1)] {
        assert_eq!((band.word(), Band::from_word(word)), (word, Some(band)));
    }
    assert_eq!(Band::from_word(2), None);
}

// ---------------------------------------------------------------- RASTER-07, the strings

#[test]
fn raster_07_refuses_an_attribution_with_a_bidi_override_a_control_or_a_line_separator() {
    for bad in [
        "evil \u{202E}gnp.exe",  // right-to-left override
        "x\u{202A}y",            // embedding
        "x\u{2066}y",            // isolate
        "line\nbreak",           // C0
        "tab\there",             // C0
        "nul\0",                 // C0
        "c1 \u{0085} next line", // C1
        "del \u{007F}",          // DEL
        "ls \u{2028} sep",
        "ps \u{2029} sep",
    ] {
        assert_eq!(rule_of(with(|p| p.attribution = bad.into())), "RASTER-07", "{bad:?}");
    }
    assert_eq!(rule_of(with(|p| p.attribution = "a".repeat(256))), "RASTER-07");
    assert!(encode_rasters(&[with(|p| p.attribution = "a".repeat(255))]).is_ok());
    // plain text in another script is fine
    assert!(
        encode_rasters(&[with(|p| p.attribution = "Plan \u{00e9}tage 2 \u{2013} \u{6771}\u{4eac}".into())])
            .is_ok()
    );
}

#[test]
fn raster_07_wants_a_digest_of_exactly_32_bytes_and_valid_utf_8_when_read() {
    let mut bytes = encode_rasters(&[with(|p| p.source_digest = Some([7; 32]))]).unwrap();
    // digest_len is at offset 85 of the record, which starts after the u32 count
    assert_eq!(bytes[4 + 84], 32);
    bytes[4 + 84] = 16;
    assert_eq!(decode_rasters(&bytes).unwrap_err().rule(), "RASTER-07");
    let mut bad = encode_rasters(&[with(|p| p.attribution = "ab".into())]).unwrap();
    let last = bad.len() - 1;
    bad[last] = 0xFF;
    assert_eq!(decode_rasters(&bad).unwrap_err().rule(), "RASTER-07");
}

// ---------------------------------------------------------------- LIMIT-13

#[test]
fn limit_13_accepts_4096_placements_and_refuses_4097() {
    let many = |n: usize| vec![vector(); n];
    let ok = encode_rasters(&many(4_096)).unwrap();
    assert_eq!(decode_rasters(&ok).unwrap().len(), 4_096);
    assert_eq!(encode_rasters(&many(4_097)).unwrap_err().rule(), "LIMIT-13");
    // the count is checked before the bytes are looked at, so a file that lies cannot cause an allocation
    let mut lie = 4_097u32.to_le_bytes().to_vec();
    lie.extend_from_slice(&[0; 16]);
    assert_eq!(decode_rasters(&lie).unwrap_err().rule(), "LIMIT-13");
    assert_eq!(decode_rasters(&u32::MAX.to_le_bytes()).unwrap_err().rule(), "LIMIT-13");
    assert_eq!(MAX_RASTERS, 4_096);
}

#[test]
fn a_raster_id_is_dense_converts_to_the_directory_cell_and_enforces_limit_13() {
    assert_eq!(RasterId::new(0).unwrap().cell(), 0);
    let last = RasterId::new(4_095).unwrap();
    assert_eq!((last.index(), last.cell()), (4_095, 4_095));
    assert_eq!(RasterId::new(4_096).unwrap_err().rule(), "LIMIT-13");
    assert_eq!(RasterId::new(u32::MAX).unwrap_err().rule(), "LIMIT-13");
}

// ---------------------------------------------------------------- framing

#[test]
fn a_damaged_payload_is_a_stream_error_with_its_comp_rule() {
    let good = encode_rasters(&[vector()]).unwrap();
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(decode_rasters(&trailing).unwrap_err().rule(), "COMP-02");
    assert_eq!(decode_rasters(&good[..good.len() - 1]).unwrap_err().rule(), "COMP-01");
    assert_eq!(decode_rasters(&[1]).unwrap_err().rule(), "COMP-01");
    let mut reserved = good.clone();
    reserved[4 + 86] = 1;
    assert_eq!(decode_rasters(&reserved).unwrap_err().rule(), "COMP-12");
    // a pixel format, a band and a bit depth outside their closed sets are undefined tags
    for (at, value) in [(76usize, 9u8), (79, 5), (77, 4)] {
        let mut bad = good.clone();
        bad[4 + at] = value;
        assert_eq!(decode_rasters(&bad).unwrap_err().rule(), "COMP-11", "offset {at}");
    }
}

#[test]
fn the_stream_is_framed_under_capability_21_and_a_set_with_it_still_decodes_when_it_is_unknown() {
    let registry = Registry::v1();
    let cap = CapabilityId::new(21);
    assert_eq!(RASTER_CAPABILITY, cap);
    let stream = Stream {
        capability: cap,
        version: registry.get(cap).expect("21 is registered").version,
        payload: encode_rasters(&[vector()]).unwrap(),
    };
    let set = encode_streams(&[stream]).unwrap();
    let knows = Reader { version: Version::new(1, 1, 0), implemented: vec![cap] };
    let opened = read_streams(&set, &registry, &knows).unwrap();
    match read_rasters(&opened) {
        RasterRead::Present(p) => assert_eq!(p, vec![vector()]),
        other => panic!("{other:?}"),
    }
    let nobody = Reader { version: Version::new(1, 0, 0), implemented: vec![] };
    let opened = read_streams(&set, &registry, &nobody).unwrap();
    assert_eq!(opened.skipped.len(), 1);
    assert_eq!(read_rasters(&opened), RasterRead::Absent);
    let damaged = Stream { payload: vec![9, 9], ..opened_stream(&set, &registry, &knows) };
    let set = encode_streams(&[damaged]).unwrap();
    let opened = read_streams(&set, &registry, &knows).unwrap();
    assert!(matches!(read_rasters(&opened), RasterRead::Damaged(_)));
}

fn opened_stream(set: &[u8], registry: &Registry, reader: &Reader) -> Stream {
    read_streams(set, registry, reader).unwrap().streams.remove(0)
}

// ---------------------------------------------------------------- the error table

#[test]
fn every_error_rule_is_in_the_table_the_document_renders() {
    let rules: Vec<&str> = RASTER_ERRORS.iter().map(|(rule, _, _)| *rule).collect();
    for want in ["LIMIT-13", "RASTER-01", "RASTER-04", "RASTER-05", "RASTER-07"] {
        assert!(rules.contains(&want), "{want} is not in RASTER_ERRORS");
    }
    for (rule, name, meaning) in RASTER_ERRORS {
        assert!(!name.is_empty() && !meaning.is_empty(), "{rule}");
    }
}
