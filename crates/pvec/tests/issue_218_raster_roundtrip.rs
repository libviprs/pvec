//! Issue #218 (R1.4): the facade writes and reads a raster through `pvec::unstable`: the placement's six
//! coefficients come back as the same bits, and every tile comes back with the same digest.
#![cfg(feature = "unstable-stream")]

#[path = "../../pvec-testkit/src/tilegen.rs"]
mod tilegen;

use pvec::unstable::{Band, PixelFormat, Placement, RasterId, TileCodec, TileIn};
use pvec::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_container::integrity::sha256;

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [2000.0, 2000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-218".into(),
        producer_version: "1".into(),
    }
}

/// A rotated, sheared, upright-in-source-space placement of a 700 by 500 image, every number exact in binary.
fn placement() -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
        width: 700,
        height: 500,
        visible_box: [10, 20, 690, 480],
        tile_edge: 256,
        levels: 3,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 200,
        band: Band::Overlay,
        layer_index: None,
        source_digest: Some([7; 32]),
        attribution: "Survey 2024".into(),
    }
}

/// Every tile of the 3 level pyramid (1 by 1, 2 by 1, then 3 by 2), the sparse one left out.
fn tiles() -> Vec<TileIn> {
    let mut out = Vec::new();
    for (l, (cols, rows)) in [(1u32, 1u32), (2, 1), (3, 2)].into_iter().enumerate() {
        for row in 0..rows {
            for col in 0..cols {
                let bytes = if (l, col, row) == (2, 2, 1) {
                    Vec::new()
                } else {
                    tilegen::png(256, 1, 8, &tilegen::payload(843, (13 * col + 29 * row + 71 * l as u32) as u8))
                };
                out.push(TileIn { level: l as u32, col, row, bytes });
            }
        }
    }
    out
}

fn write() -> Vec<u8> {
    let mut w = PvecWriter::new(manifest()).unwrap();
    let line = Curve::Polyline(
        pvec_geometry::Polyline::new(vec![
            pvec_geometry::Point2 { x: 0.0, y: 0.0 },
            pvec_geometry::Point2 { x: 400.0, y: 300.0 },
        ])
        .unwrap(),
    );
    w.push(EntityId::new(0).unwrap(), &line).unwrap();
    w.rasters(&[placement()]).unwrap();
    w.raster_tiles(RasterId::new(0).unwrap(), tiles()).unwrap();
    w.finish().unwrap()
}

#[test]
fn the_placement_comes_back_with_the_same_bits() {
    let r = PvecReader::open(write()).unwrap();
    let got = r.raster_placements().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], placement());
    for (a, b) in got[0].pixel_to_source.iter().zip(placement().pixel_to_source) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
}

#[test]
fn every_tile_comes_back_with_the_same_digest_and_a_tile_not_stored_comes_back_as_nothing() {
    let r = PvecReader::open(write()).unwrap();
    for t in tiles() {
        let got = r.raster_tile(RasterId::new(0).unwrap(), t.level, t.col, t.row).unwrap();
        if t.bytes.is_empty() {
            assert_eq!(got, None);
        } else {
            assert_eq!(sha256(&got.expect("stored")), sha256(&t.bytes));
        }
    }
}

#[test]
fn the_second_write_is_byte_identical_to_the_first() {
    assert_eq!(write(), write());
}

#[test]
fn an_indexed_writer_lays_the_same_raster_out_and_the_geometry_still_reads() {
    let mut w = PvecWriter::new_indexed(manifest(), 64, None).unwrap();
    let line = Curve::Polyline(
        pvec_geometry::Polyline::new(vec![
            pvec_geometry::Point2 { x: 0.0, y: 0.0 },
            pvec_geometry::Point2 { x: 400.0, y: 300.0 },
        ])
        .unwrap(),
    );
    w.push(EntityId::new(0).unwrap(), &line).unwrap();
    w.rasters(&[placement()]).unwrap();
    w.raster_tiles(RasterId::new(0).unwrap(), tiles()).unwrap();
    let r = PvecReader::open(w.finish().unwrap()).unwrap();
    assert_eq!(r.raster_placements().unwrap(), vec![placement()]);
    assert!(r.raster_tile(RasterId::new(0).unwrap(), 2, 0, 0).unwrap().is_some());
}

#[test]
fn a_file_with_no_raster_has_no_placements_and_no_tile() {
    let mut w = PvecWriter::new(manifest()).unwrap();
    let line = Curve::Polyline(
        pvec_geometry::Polyline::new(vec![
            pvec_geometry::Point2 { x: 0.0, y: 0.0 },
            pvec_geometry::Point2 { x: 4.0, y: 3.0 },
        ])
        .unwrap(),
    );
    w.push(EntityId::new(0).unwrap(), &line).unwrap();
    let r = PvecReader::open(w.finish().unwrap()).unwrap();
    assert!(r.raster_placements().unwrap().is_empty());
    assert!(r.raster_tile(RasterId::new(0).unwrap(), 0, 0, 0).is_err());
}
