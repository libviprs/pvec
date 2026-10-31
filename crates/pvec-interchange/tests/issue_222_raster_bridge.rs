//! The PMTiles raster passthrough, container layer (issue R2.3, pvec#222): a raster of a `.pvec` out to an
//! archive and an archive plus a placement back in, kinds 9 and 10 and a stream entry, bytes unchanged.
//!
//! Red first: these describe `pvec_interchange::pmtiles::{export_raster, import_raster}` before they exist.
//! The archive is read by `tests/pmtiles_ref`, which shares no code with the writer.

mod pmtiles_ref;
mod report_json;

use pmtiles_ref::*;
use pvec_capabilities::Registry;
use pvec_components::{
    Band, PixelFormat, Placement, RASTER_CAPABILITY, RasterId, Stream, TileCodec, encode_rasters,
};
use pvec_container::blocks::raster::{TileIn, grid_of};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_interchange::pmtiles::{
    ImportError, RasterArchive, RasterReadLimits, RasterTile, RasterTileType, export_raster, import_raster,
    placement_from_json, placement_json, read_raster_archive, write_raster_archive,
};
use pvec_interchange::report::{RasterOutcome, RasterRow};
use pvec_testkit::{raster_hostile, tilegen};
use report_json::json;

// ---- fixtures -------------------------------------------------------------------------------------------

/// A 1000 by 600 image at edge 256: three levels, 4 by 3 tiles at level 2, 2 by 2 at level 1, 1 by 1 at 0.
/// The affine is rotated and sheared, and has bits a JSON number would round.
fn placement() -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.1, 50.3],
        width: 1000,
        height: 600,
        visible_box: [10, 20, 990, 590],
        tile_edge: 256,
        levels: 3,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Rgba,
        bit_depth: 8,
        opacity: 200,
        band: Band::Overlay,
        layer_index: Some(3),
        source_digest: Some([7; 32]),
        attribution: "Map data <b>& co</b>".into(),
    }
}

fn tile_bytes(level: u32, col: u32, row: u32) -> Vec<u8> {
    let seed = (level * 16 + col * 4 + row) as u8;
    tilegen::png(256, 4, 8, &tilegen::payload(300 + (level + col + row) as usize, seed))
}

/// Every tile of the pyramid except the ones `missing` names, which stay not stored.
fn tiles(p: &Placement, missing: &[(u32, u32, u32)]) -> Vec<TileIn> {
    let mut out = Vec::new();
    for level in 0..u32::from(p.levels) {
        let (cols, rows) = grid_of(p.width, p.height, 256, u32::from(p.levels), level);
        for row in 0..rows {
            for col in 0..cols {
                if !missing.contains(&(level, col, row)) {
                    out.push(TileIn { level, col, row, bytes: tile_bytes(level, col, row) });
                }
            }
        }
    }
    out
}

fn container(p: &Placement, tiles: Vec<TileIn>) -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "issue-222".into(),
            producer_version: "0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    let bounds = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
    w.push_line(EntityMeta { id: EntityId::new(0).unwrap(), style: 0, cell: 0, bounds }, [0.0; 2], [1.0; 2])
        .unwrap();
    let version = Registry::v1().get(RASTER_CAPABILITY).unwrap().version;
    let payload = encode_rasters(std::slice::from_ref(p)).unwrap();
    w.push_stream(Stream { capability: RASTER_CAPABILITY, version, payload }).unwrap();
    w.push_raster_tiles(RasterId::new(0).unwrap(), tiles).unwrap();
    w.finish().unwrap()
}

fn open(bytes: Vec<u8>) -> ContainerReader<MemorySource<Vec<u8>>> {
    ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap()
}

fn same_bits(a: &Placement, b: &Placement) -> bool {
    encode_rasters(std::slice::from_ref(a)).unwrap() == encode_rasters(std::slice::from_ref(b)).unwrap()
}

const MISSING: [(u32, u32, u32); 3] = [(2, 1, 0), (2, 3, 2), (1, 1, 0)];

fn exported() -> (Vec<u8>, Vec<u8>) {
    let p = placement();
    let file = container(&p, tiles(&p, &MISSING));
    let archive = export_raster(&open(file.clone()), 0).expect("the raster exports");
    (file, archive)
}

// ---- out ------------------------------------------------------------------------------------------------

#[test]
fn issue_222_the_directory_has_one_entry_per_stored_tile_and_none_for_a_not_stored_one() {
    let (_, a) = exported();
    let stored = 12 + 3 + 1;
    assert_eq!(all_tiles(&a).len(), stored);
    for (l, c, r) in MISSING {
        assert!(find(&a, ref_tile_id(l, u64::from(c), u64::from(r))).is_none(), "{l}/{c}/{r}");
    }
    let h = header(&a);
    assert_eq!((h.addressed, h.tile_type, h.tile_compression), (stored as u64, 2, 1));
    assert_eq!((h.min_zoom, h.max_zoom), (0, 2));
}

#[test]
fn issue_222_z_x_y_are_level_column_row_and_every_tile_is_the_stored_one() {
    let (_, a) = exported();
    for t in tiles(&placement(), &MISSING) {
        let id = ref_tile_id(t.level, u64::from(t.col), u64::from(t.row));
        assert_eq!(find(&a, id).unwrap(), &t.bytes[..], "{}/{}/{}", t.level, t.col, t.row);
    }
}

#[test]
fn issue_222_the_tile_type_follows_the_codec() {
    for (codec, format, byte) in [
        (TileCodec::Png, PixelFormat::Rgba, 2u8),
        (TileCodec::Jpeg, PixelFormat::Rgb, 3),
        (TileCodec::Webp, PixelFormat::Rgba, 4),
    ] {
        let p = Placement {
            width: 256,
            height: 256,
            visible_box: [0, 0, 256, 256],
            levels: 1,
            tile_codec: codec,
            pixel_format: format,
            ..placement()
        };
        let bytes = match codec {
            TileCodec::Png => tilegen::png(256, 4, 8, &tilegen::payload(50, 1)),
            TileCodec::Jpeg => tilegen::jpeg(256, 3, false, &tilegen::payload(50, 1)),
            _ => tilegen::webp_vp8l(256, 256, &tilegen::payload(50, 1)),
        };
        let file = container(&p, vec![TileIn { level: 0, col: 0, row: 0, bytes: bytes.clone() }]);
        let a = export_raster(&open(file), 0).unwrap();
        assert_eq!(header(&a).tile_type, byte);
        assert_eq!(all_tiles(&a)[0].1, bytes);
    }
}

#[test]
fn issue_222_the_placement_key_and_the_attribution_are_in_the_metadata_and_round_trip() {
    let (_, a) = exported();
    let m = json(std::str::from_utf8(range(&a, header(&a).meta)).unwrap());
    assert_eq!(m.get("attribution").str(), "Map data <b>& co</b>", "plain text, never escaped into HTML");
    let r = m.get("vnd.pvec").get("raster");
    let bits = |i: usize| r.get("pixel_to_source").arr()[i].str().to_string();
    assert_eq!(bits(0), format!("{:016x}", 0.5f64.to_bits()));
    assert_eq!(bits(4), format!("{:016x}", 100.1f64.to_bits()), "hex of the bits, so no number rounds it");
    assert_eq!(r.get("width").num(), 1000.0);
    assert_eq!(r.get("tile_edge").num(), 256.0);
    let back = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    assert_eq!(back.tile_type, RasterTileType::Png);
    assert!(back.pvec.is_some());
}

#[test]
fn issue_222_the_same_raster_gives_the_same_archive() {
    assert_eq!(exported().1, exported().1);
}

// ---- in -------------------------------------------------------------------------------------------------

#[test]
fn issue_222_an_archive_comes_back_with_every_tile_by_digest_and_the_placement_by_bits() {
    let (_, a) = exported();
    for given in [None, Some(placement())] {
        let got = import_raster(&a, given.as_ref(), &RasterReadLimits::default()).expect("imports");
        assert!(same_bits(&got.placement, &placement()), "the six coefficients and every field, bit for bit");
        let mut want = tiles(&placement(), &MISSING);
        let mut have = got.tiles;
        want.sort_by_key(|t| (t.level, t.row, t.col));
        have.sort_by_key(|t| (t.level, t.row, t.col));
        assert_eq!(have, want);
    }
}

#[test]
fn issue_222_export_then_import_then_write_gives_the_file_back_byte_for_byte() {
    let (file, a) = exported();
    let got = import_raster(&a, None, &RasterReadLimits::default()).unwrap();
    assert_eq!(container(&got.placement, got.tiles), file);
}

#[test]
fn issue_222_a_placement_that_disagrees_with_the_archive_or_no_placement_at_all_is_refused() {
    let (_, a) = exported();
    let other = Placement { opacity: 10, ..placement() };
    let e = import_raster(&a, Some(&other), &RasterReadLimits::default()).unwrap_err();
    assert!(matches!(e, ImportError::PlacementDisagrees), "{e}");

    let bare = write_raster_archive(&RasterArchive {
        tile_type: RasterTileType::Png,
        tiles: vec![RasterTile { z: 0, x: 0, y: 0, bytes: tile_bytes(0, 0, 0) }],
        pvec: None,
        attribution: None,
    })
    .unwrap();
    let e = import_raster(&bare, None, &RasterReadLimits::default()).unwrap_err();
    assert!(matches!(e, ImportError::NoPlacement), "{e}");
}

#[test]
fn issue_222_a_placement_in_the_archive_is_validated_like_a_stream_row() {
    let (_, a) = exported();
    let text = String::from_utf8_lossy(range(&a, header(&a).meta)).to_string();
    // the same archive with levels 3 changed to 9: RASTER-01
    let bad = text.replace("\"levels\":3", "\"levels\":9");
    assert_ne!(bad, text);
    let archive = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    let json_text = archive.pvec.unwrap().replace("\"levels\":3", "\"levels\":9");
    let forged = write_raster_archive(&RasterArchive { pvec: Some(json_text), ..archive }).unwrap();
    let e = import_raster(&forged, None, &RasterReadLimits::default()).unwrap_err();
    assert_eq!(e.rule(), "RASTER-01", "{e}");
}

#[test]
fn issue_222_a_raw_or_vector_tile_type_is_refused() {
    let (_, a) = exported();
    for ty in [0u8, 1] {
        let mut b = a.clone();
        b[99] = ty;
        assert!(import_raster(&b, None, &RasterReadLimits::default()).is_err(), "tile type {ty}");
    }
}

#[test]
fn issue_222_a_tile_codec_that_is_not_the_placements_is_refused() {
    let (_, a) = exported();
    let mut b = a.clone();
    b[99] = 3; // JPEG under a PNG placement
    let e = import_raster(&b, None, &RasterReadLimits::default()).unwrap_err();
    assert!(matches!(e, ImportError::CodecDisagrees), "{e}");
}

#[test]
fn issue_222_every_hostile_tile_is_refused_with_its_rule_and_nothing_is_returned() {
    let mut refused = 0;
    for t in raster_hostile::hostile() {
        if !(1..=3).contains(&t.codec) || t.bytes.is_empty() {
            continue; // an unknown codec has no PMTiles type; an empty tile is not stored
        }
        let ty = [RasterTileType::Png, RasterTileType::Jpeg, RasterTileType::Webp][usize::from(t.codec) - 1];
        let a = write_raster_archive(&RasterArchive {
            tile_type: ty,
            tiles: vec![RasterTile { z: 0, x: 0, y: 0, bytes: t.bytes.clone() }],
            pvec: Some(placement_json(&raster_hostile::placement(&t))),
            attribution: None,
        })
        .unwrap();
        let e = import_raster(&a, None, &RasterReadLimits::default()).unwrap_err();
        assert_eq!(Some(e.rule()), t.rule, "{}: {e}", t.name);
        refused += 1;
    }
    assert!(refused >= 8, "{refused} hostile tiles were driven through the importer");
}

#[test]
fn issue_222_the_accepted_tiles_import() {
    for t in raster_hostile::accepted() {
        let ty = [RasterTileType::Png, RasterTileType::Jpeg, RasterTileType::Webp][usize::from(t.codec) - 1];
        let a = write_raster_archive(&RasterArchive {
            tile_type: ty,
            tiles: vec![RasterTile { z: 0, x: 0, y: 0, bytes: t.bytes.clone() }],
            pvec: Some(placement_json(&raster_hostile::placement(&t))),
            attribution: None,
        })
        .unwrap();
        let got = import_raster(&a, None, &RasterReadLimits::default());
        assert!(got.is_ok(), "{}: {:?}", t.name, got.err());
    }
}

#[test]
fn issue_222_the_edge_is_sniffed_from_the_tile_and_a_tile_that_says_another_edge_is_refused() {
    let p = Placement {
        width: 256,
        height: 256,
        visible_box: [0, 0, 256, 256],
        levels: 1,
        ..placement()
    };
    let with_edge = |edge: u32| {
        let a = write_raster_archive(&RasterArchive {
            tile_type: RasterTileType::Png,
            tiles: vec![RasterTile { z: 0, x: 0, y: 0, bytes: tilegen::png(edge, 4, 8, &tilegen::payload(40, 1)) }],
            pvec: Some(placement_json(&p)),
            attribution: None,
        })
        .unwrap();
        import_raster(&a, None, &RasterReadLimits::default())
    };
    assert!(with_edge(256).is_ok());
    for edge in [300, 512, 255] {
        let e = with_edge(edge).unwrap_err();
        assert_eq!(e.rule(), "RASTER-03", "a tile of {edge}: {e}");
    }
}

#[test]
fn issue_222_the_placement_text_round_trips_through_its_conversion() {
    let p = placement();
    let back = placement_from_json(&placement_json(&p)).unwrap();
    assert!(same_bits(&back, &p));
    assert_eq!(placement_json(&back), placement_json(&p));
}

#[test]
fn issue_222_a_tile_outside_the_pyramid_is_refused() {
    let (_, a) = exported();
    let mut ar = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    ar.tiles.push(RasterTile { z: 3, x: 0, y: 0, bytes: tile_bytes(0, 0, 0) });
    let e = import_raster(&write_raster_archive(&ar).unwrap(), None, &RasterReadLimits::default()).unwrap_err();
    assert!(matches!(e, ImportError::TileOutsidePyramid { .. }), "{e}");
    let mut ar = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    ar.tiles.push(RasterTile { z: 0, x: 1, y: 0, bytes: tile_bytes(0, 0, 0) });
    let e = import_raster(&write_raster_archive(&ar).unwrap(), None, &RasterReadLimits::default()).unwrap_err();
    assert!(matches!(e, ImportError::TileOutsidePyramid { .. }), "{e}");
}

// ---- the report -----------------------------------------------------------------------------------------

#[test]
fn issue_222_a_refusal_is_a_report_line_that_names_the_rule() {
    let (_, a) = exported();
    let mut ar = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    ar.tiles[0].bytes = tilegen::png(255, 4, 8, &tilegen::payload(40, 1));
    let e = import_raster(&write_raster_archive(&ar).unwrap(), None, &RasterReadLimits::default()).unwrap_err();
    let row = RasterRow::refused(0, &e);
    match &row.outcome {
        RasterOutcome::Unsupported(r) => assert!(r.as_str().contains("RASTER-03"), "{}", r.as_str()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn issue_222_exporting_a_raster_the_file_does_not_have_is_an_error_not_an_empty_archive() {
    let (file, _) = exported();
    assert!(export_raster(&open(file), 5).is_err());
}

#[test]
fn issue_222_an_exported_tile_that_fails_the_sniffers_refuses_the_whole_raster() {
    let t = raster_hostile::hostile().into_iter().find(|t| t.name == "header-lies-about-the-size").unwrap();
    let file = raster_hostile::file(&t);
    let e = export_raster(&open(file), 0).unwrap_err();
    assert_eq!(e.rule(), "RASTER-03", "{e}");
}
