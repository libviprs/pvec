//! Issue #271: the file the viewer's browser tests open, `tests/compat/viewer-page.pvec`. It is shaped like a
//! converted PDF page: source units are points with y up, a 612 by 792 frame, some linework, and one RGB raster
//! placed the way a PDF image is (pixel rows down, source y up). The specs in `viewer/tests/` know these
//! numbers, so a change here is a change there. `PVEC_BLESS_FIXTURE=1` writes the file again. The manifest unit is
//! metres with no scale, which is what libviprs writes for a PDF today (points cannot be declared, `pdf.rs`); the
//! calibration block is where a real scale goes.

use pvec_components::{Band, PixelFormat, Placement, TileCodec};
use pvec_container::blocks::raster::TileIn;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_testkit::raster_file::{RasterIn, raster_file};
use pvec_testkit::tilegen;
use pvec_testkit::tiler::Shape;
use pvec_wasm::Runtime;
use std::f64::consts::TAU;
use std::fs;
use std::path::Path;

#[test]
fn issue_271_the_viewer_fixture_is_what_this_test_writes_and_the_module_reads_it() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/viewer-page.pvec");
    let want = {
        let mut w = ContainerWriter::new(WriterConfig {
            unit: LengthUnit::Metre,
            metadata: Metadata {
                producer_id: "pvec-viewer-fixture".into(),
                producer_version: "0".into(),
                corpus_revision: None,
            },
            bounds: None,
            capabilities: Vec::new(),
        });
        let meta = |id: u32, min: [f64; 2], max: [f64; 2]| EntityMeta {
            id: EntityId::new(id).expect("an id"),
            style: 0,
            cell: 0,
            bounds: Bounds { min, max },
        };
        // the page frame
        w.push_polyline(
            meta(0, [0.0, 0.0], [612.0, 792.0]),
            &[[0.0, 0.0], [612.0, 0.0], [612.0, 792.0], [0.0, 792.0], [0.0, 0.0]],
        )
        .expect("the frame");
        // a corner mark at the top left only, so a flipped y shows
        w.push_polyline(
            meta(1, [20.0, 712.0], [80.0, 772.0]),
            &[[20.0, 772.0], [80.0, 772.0], [80.0, 712.0]],
        )
        .expect("the mark");
        // a line across the raster, which must draw over it
        w.push_line(meta(2, [36.0, 550.0], [576.0, 550.0]), [36.0, 550.0], [576.0, 550.0]).expect("the line");
        // a full circle below the raster
        w.push_circle_arc(meta(3, [226.0, 120.0], [386.0, 280.0]), [306.0, 200.0], 80.0, [0.0, TAU])
            .expect("the circle");

        // 512 by 256 RGB, left half one colour and right half another, on 400 by 200 points from (100, 450)
        let shape = Shape { width: 512, height: 256, tile_edge: 256, channels: 3, bit_depth: 8 };
        let levels = shape.levels();
        let mut tiles = Vec::new();
        for (l, (cols, rows)) in shape.grid().into_iter().enumerate() {
            let scale = 1u32 << (levels - 1 - l as u32);
            for row in 0..rows {
                for col in 0..cols {
                    let mut raw = Vec::with_capacity(256 * (1 + 256 * 3));
                    for y in 0..256 {
                        raw.push(0);
                        for x in 0..256 {
                            let (px, py) = ((col * 256 + x) * scale, (row * 256 + y) * scale);
                            raw.extend_from_slice(match (px < 512 && py < 256, px < 256) {
                                (false, _) => &[0, 0, 0],
                                (true, true) => &[0xD0, 0x40, 0x30],
                                (true, false) => &[0x30, 0x60, 0xD0],
                            });
                        }
                    }
                    let bytes = tilegen::png(256, 3, 8, &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9));
                    tiles.push(TileIn { level: l as u32, col, row, bytes });
                }
            }
        }
        let placement = Placement {
            pixel_to_source: [0.78125, 0.0, 0.0, -0.78125, 100.0, 650.0],
            width: 512,
            height: 256,
            visible_box: [0, 0, 512, 256],
            tile_edge: 256,
            levels: levels as u8,
            tile_codec: TileCodec::Png,
            pixel_format: PixelFormat::Rgb,
            bit_depth: 8,
            opacity: 255,
            band: Band::Underlay,
            layer_index: None,
            source_digest: None,
            attribution: "viewer fixture".into(),
        };
        raster_file(w, &[RasterIn { placement, tiles, inline_levels: 1, chunk_bytes: 1 << 20 }], 1 << 20)
    };
    if std::env::var_os("PVEC_BLESS_FIXTURE").is_some() {
        fs::write(&path, &want).unwrap();
    }
    let have = fs::read(&path).expect("tests/compat/viewer-page.pvec: write it with PVEC_BLESS_FIXTURE=1");
    assert!(have == want, "tests/compat/viewer-page.pvec is not what this test writes: bless it again");
    let mut rt = Runtime::new();
    let f = rt.open(have).expect("the module opens it");
    assert_eq!((rt.raster_stat(f, 0), rt.raster_stat(f, 1)), (Ok(1), Ok(0)), "one raster, none skipped");
    let batch = rt
        .query_viewport(f, pvec::Bounds2 { min: [-10_000.0, -10_000.0], max: [10_000.0, 10_000.0] }, 1 << 20)
        .expect("a batch");
    assert_eq!(rt.batch_len(batch), Ok(4), "the frame, the mark, the line and the circle");
}
