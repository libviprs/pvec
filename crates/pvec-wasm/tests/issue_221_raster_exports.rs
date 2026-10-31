//! Issue #221: the raster exports at ABI version 2. A `Host` plays the browser as in issue 205: it owns the
//! bytes and answers what the module says it needs. The file is the test-only raster file, so the bytes a tile
//! call returns are compared with the bytes the tiler wrote. `docs/WASM_ABI.md` is the contract.

use pvec::PvecReader;
use pvec::unstable::{Aabb, ViewportQuery};
use pvec_components::{Band, PixelFormat, Placement, TileCodec};
use pvec_container::blocks::raster::{KIND_INDEX, KIND_TILES, TileIn};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_testkit::raster_file::{RasterIn, line_writer, raster_file};
use pvec_testkit::tiler::{Shape, tiles};
use pvec_wasm::{AbiError, Runtime, WASM_ABI_VERSION};
use std::fs;
use std::path::Path;

const SHAPE: Shape = Shape { width: 1024, height: 512, tile_edge: 256, channels: 1, bit_depth: 8 };
const WHOLE: [f64; 4] = [-1000.0, -1000.0, 1000.0, 1000.0];

fn placement(codec: TileCodec, layer: Option<u32>) -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
        width: 1024,
        height: 512,
        visible_box: [0, 0, 1024, 512],
        tile_edge: 256,
        levels: 3,
        tile_codec: codec,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 200,
        band: Band::Overlay,
        layer_index: layer,
        source_digest: Some([0x5A; 32]),
        attribution: "survey 1998".into(),
    }
}

/// Raster 0: whole, level 0 inline. Raster 1: tile (3, 1) of the full resolution level is not stored. Raster 2: a
/// tile codec this build does not know, so it is skipped (RASTER-04).
fn rasters() -> Vec<RasterIn> {
    let all = tiles(SHAPE, |_, _, _| true);
    let sparse = tiles(SHAPE, |level, col, row| !(level == 2 && (col, row) == (3, 1)));
    let mk =
        |placement, tiles, inline_levels| RasterIn { placement, tiles, inline_levels, chunk_bytes: 1 << 21 };
    vec![
        mk(placement(TileCodec::Png, Some(2)), all.clone(), 1),
        mk(placement(TileCodec::Png, None), sparse, 0),
        mk(placement(TileCodec::Unknown(9), None), all, 0),
    ]
}

fn file() -> Vec<u8> {
    raster_file(line_writer(), &rasters(), 1 << 20)
}

fn window(b: [f64; 4]) -> pvec::Bounds2 {
    pvec::Bounds2 { min: [b[0], b[1]], max: [b[2], b[3]] }
}

/// The browser: bytes, and a log of what it was asked for.
struct Host {
    bytes: Vec<u8>,
    served: Vec<(u64, u32)>,
}

impl Host {
    fn new(bytes: Vec<u8>) -> Host {
        Host { bytes, served: Vec::new() }
    }

    fn serve(&mut self, rt: &mut Runtime, handle: u32) {
        for (offset, len) in rt.needed(handle).unwrap() {
            self.served.push((offset, len));
            let end = (offset + u64::from(len)) as usize;
            rt.provide_range(handle, offset, &self.bytes[offset as usize..end]).unwrap();
        }
    }

    fn open(&mut self, rt: &mut Runtime) -> u32 {
        let h = rt.open_ranged(self.bytes.len() as u64).unwrap();
        loop {
            match rt.open_poll(h) {
                Ok(f) => return f,
                Err(AbiError::NeedRange) => self.serve(rt, h),
                Err(e) => panic!("open failed: {e:?}"),
            }
        }
    }

    fn run<T>(
        &mut self,
        rt: &mut Runtime,
        handle: u32,
        mut call: impl FnMut(&mut Runtime) -> Result<T, AbiError>,
    ) -> Result<T, AbiError> {
        loop {
            match call(rt) {
                Err(AbiError::NeedRange) => self.serve(rt, handle),
                other => return other,
            }
        }
    }
}

fn open_whole(bytes: Vec<u8>) -> (Runtime, u32) {
    let mut rt = Runtime::new();
    let f = rt.open(bytes).unwrap();
    (rt, f)
}

#[test]
fn issue_221_the_abi_stays_2_and_the_document_names_every_export_and_code() {
    assert_eq!(WASM_ABI_VERSION, 2);
    assert_eq!(AbiError::NoRaster.code(), -8);
    assert_eq!(AbiError::TileNotStored.code(), -9);
    assert_eq!(AbiError::RasterSkipped.code(), -10);
    // -3 and -4 keep their meaning
    assert_eq!(AbiError::BadArgument.code(), -3);
    assert_eq!(AbiError::Read.code(), -4);
    let doc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/WASM_ABI.md")).unwrap();
    for needle in [
        "raster_stat",
        "raster_skipped",
        "raster_placement",
        "raster_plan",
        "raster_tile",
        "raster_buffer_len",
        "raster_buffer_ptr",
        "raster_buffer_elem",
        "Uint8Array",
        "-8",
        "-9",
        "-10",
        "issue_221_raster_exports",
    ] {
        assert!(doc.contains(needle), "docs/WASM_ABI.md does not mention {needle}");
    }
    let bindings = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bindings.rs")).unwrap();
    for export in [
        "raster_stat",
        "raster_skipped",
        "raster_placement",
        "raster_plan",
        "raster_tile",
        "raster_buffer_len",
        "raster_buffer_ptr",
        "raster_buffer_elem",
    ] {
        assert!(bindings.contains(&format!("pub fn {export}(")), "no export {export}");
    }
}

#[test]
fn issue_221_the_stats_of_the_fixture_are_right() {
    let (mut rt, f) = open_whole(file());
    assert_eq!(rt.raster_stat(f, 0), Ok(3), "rasters");
    assert_eq!(rt.raster_stat(f, 1), Ok(1), "skipped");
    assert_eq!(rt.raster_stat(f, 2), Ok(1), "an image this reader cannot show");
    assert_eq!(rt.raster_stat(f, 3), Ok(1), "has a stream");
    assert_eq!(rt.raster_stat(f, 4), Err(AbiError::BadArgument));
    // the skipped raster and its reason: RASTER-04, an unknown tile codec
    assert_eq!(rt.raster_skipped(f, 0, 0), Ok(2));
    assert_eq!(rt.raster_skipped(f, 0, 1), Ok(4));
    assert_eq!(rt.raster_skipped(f, 1, 0), Err(AbiError::BadArgument));
}

#[test]
fn issue_221_a_file_with_no_raster_stream_says_so() {
    let mut w = line_writer();
    let _ = &mut w;
    let (mut rt, f) = open_whole(w.finish().unwrap());
    assert_eq!(rt.raster_stat(f, 0), Ok(0));
    assert_eq!(rt.raster_stat(f, 3), Ok(0));
    assert_eq!(rt.raster_stat(f, 2), Ok(0), "nothing to show is not an image that cannot be shown");
    assert_eq!(rt.raster_placement(f, 0), Err(AbiError::NoRaster));
    assert_eq!(rt.raster_plan(f, window(WHOLE), 1.0, 0.0), Err(AbiError::NoRaster));
    assert_eq!(rt.raster_tile(f, 0, 0, 0, 0), Err(AbiError::NoRaster));
}

#[test]
fn issue_221_the_placement_buffers_carry_the_authored_bits() {
    let (mut rt, f) = open_whole(file());
    let h = rt.raster_placement(f, 0).unwrap();
    assert_eq!(rt.raster_f64(h, 0).unwrap(), &[0.5, 0.25, 0.125, -0.5, 100.0, 50.0]);
    // width, height, visible_box x4, tile_edge, levels, tile_codec, channels, bit_depth, opacity, band, layer
    let fields = rt.raster_u32(h, 1).unwrap();
    assert_eq!(fields, &[1024, 512, 0, 0, 1024, 512, 256, 3, 1, 1, 8, 200, 1, 2]);
    assert_eq!(rt.raster_u8(h, 2).unwrap(), b"survey 1998");
    assert_eq!(rt.raster_u8(h, 3).unwrap(), &[0x5A; 32]);
    let none = rt.raster_placement(f, 1).unwrap();
    assert_eq!(rt.raster_u32(none, 1).unwrap()[13], u32::MAX, "no layer");
    assert_eq!(rt.raster_placement(f, 3), Err(AbiError::BadArgument));
    assert_eq!(rt.raster_view(h, 2).unwrap().elem, pvec_wasm::ElemType::U8);
}

fn all_tiles(r: &RasterIn) -> &[TileIn] {
    &r.tiles
}

#[test]
fn issue_221_a_tile_is_the_bytes_the_tiler_wrote() {
    let (mut rt, f) = open_whole(file());
    let r = &rasters()[0];
    let mut checked = 0;
    for t in all_tiles(r) {
        let h = rt.raster_tile(f, 0, t.level, t.col, t.row).unwrap();
        assert_eq!(rt.raster_u8(h, 0).unwrap(), &t.bytes[..], "tile {:?}", (t.level, t.col, t.row));
        assert_eq!(rt.raster_view(h, 0).unwrap().elem, pvec_wasm::ElemType::U8);
        rt.release(h).unwrap();
        checked += 1;
    }
    assert_eq!(checked, 11, "every tile of the three levels, the inline one included");
}

#[test]
fn issue_221_a_tile_that_is_not_stored_and_a_bad_address_have_their_codes() {
    let (mut rt, f) = open_whole(file());
    assert_eq!(rt.raster_tile(f, 1, 2, 3, 1), Err(AbiError::TileNotStored));
    assert!(rt.raster_tile(f, 1, 2, 2, 1).is_ok());
    assert_eq!(rt.raster_tile(f, 1, 2, 9, 9), Err(AbiError::BadArgument), "outside the grid");
    assert_eq!(rt.raster_tile(f, 1, 9, 0, 0), Err(AbiError::BadArgument), "no such level");
    assert_eq!(rt.raster_tile(f, 7, 0, 0, 0), Err(AbiError::BadArgument), "no such raster");
}

#[test]
fn issue_221_a_raster_skipped_by_a_rule_is_counted_with_its_reason_and_refused() {
    let (mut rt, f) = open_whole(file());
    assert_eq!(rt.raster_tile(f, 2, 0, 0, 0), Err(AbiError::RasterSkipped));
    let plan = rt.raster_plan(f, window(WHOLE), 1.0, 0.0).unwrap();
    assert!(!rt.raster_u32(plan, 0).unwrap().contains(&2), "a skipped raster is not planned");
}

#[test]
fn issue_221_a_flipped_byte_in_a_tile_is_a_failed_checksum() {
    let bytes = file();
    let probe = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default()).unwrap();
    let chunk = probe
        .entries()
        .iter()
        .find(|e| e.kind == KIND_TILES && e.cell == 0 && e.subtype == 2 * 1024)
        .expect("the full resolution chunk of raster 0");
    // the middle of the chunk is inside a tile
    let mut bad = bytes.clone();
    bad[chunk.offset as usize + chunk.stored_length as usize / 2] ^= 0x40;
    let (mut rt, f) = open_whole(bad);
    let mut failed = 0;
    for t in tiles(SHAPE, |l, _, _| l == 2) {
        match rt.raster_tile(f, 0, t.level, t.col, t.row) {
            Ok(_) => {}
            Err(AbiError::Read) => failed += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!(failed, 1, "exactly the tile that holds the flipped byte fails with -4");
}

#[test]
fn issue_221_the_plan_call_is_the_plan_of_pvec_stream() {
    let bytes = file();
    let (mut rt, f) = open_whole(bytes.clone());
    // the footprint of the vector: x 100 to 740, y -206 to 178; a window over half of it at half the pixels
    for (win, scale) in
        [(WHOLE, 1.0), ([100.0, -206.0, 420.0, 0.0], 1.0), ([100.0, -206.0, 740.0, 178.0], 0.25)]
    {
        let h = rt.raster_plan(f, window(win), scale, 0.0).unwrap();
        let reader = PvecReader::open(bytes.clone()).unwrap();
        let placements = reader.raster_placements().unwrap().unwrap();
        let q = ViewportQuery::new(Aabb::new([win[0], win[1]], [win[2], win[3]]).unwrap(), scale);
        let ids = rt.raster_u32(h, 0).unwrap().to_vec();
        assert_eq!(ids, vec![0, 1], "the skipped raster is not in the plan");
        let (table, ranges_table) =
            (rt.raster_u32(h, 2).unwrap().to_vec(), rt.raster_u32(h, 9).unwrap().to_vec());
        for (n, id) in ids.iter().enumerate() {
            let plan = reader.raster_plan(*id, &placements[*id as usize], &q, None).unwrap();
            assert_eq!(rt.raster_u32(h, 1).unwrap()[n], plan.level, "level of raster {id}");
            let (a, b) = (table[n] as usize, table[n + 1] as usize);
            assert_eq!(b - a, plan.tiles.len(), "tiles of raster {id}");
            for (k, t) in plan.tiles.iter().enumerate() {
                let at = a + k;
                assert_eq!(&rt.raster_u32(h, 3).unwrap()[2 * at..2 * at + 2], &[t.col, t.row]);
                match t.slot {
                    pvec::unstable::TileSlot::Stored { block, offset, len } => {
                        assert_eq!(rt.raster_u32(h, 4).unwrap()[at], block.0);
                        assert_eq!(rt.raster_f64(h, 5).unwrap()[at], offset as f64);
                        assert_eq!(rt.raster_u32(h, 6).unwrap()[at], len);
                        assert_eq!(rt.raster_u32(h, 7).unwrap()[at], 0);
                    }
                    _ => {
                        assert_eq!(rt.raster_u32(h, 4).unwrap()[at], u32::MAX);
                        assert_eq!(rt.raster_u32(h, 7).unwrap()[at], 1);
                    }
                }
            }
            let (ra, rb) = (ranges_table[n] as usize, ranges_table[n + 1] as usize);
            let got: Vec<(f64, f64)> = (ra..rb)
                .map(|i| (rt.raster_f64(h, 8).unwrap()[2 * i], rt.raster_f64(h, 8).unwrap()[2 * i + 1]))
                .collect();
            let want: Vec<(f64, f64)> = plan.ranges.iter().map(|(o, l)| (*o as f64, f64::from(*l))).collect();
            assert_eq!(got, want, "ranges of raster {id}");
        }
        rt.release(h).unwrap();
    }
}

#[test]
fn issue_221_on_a_range_opened_file_a_tile_costs_its_own_range_and_never_the_chunk() {
    let bytes = file();
    let probe = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default()).unwrap();
    let index_block = probe.entries().iter().find(|e| e.kind == KIND_INDEX && e.cell == 0).unwrap();
    let chunk = probe
        .entries()
        .iter()
        .find(|e| e.kind == KIND_TILES && e.cell == 0 && e.subtype == 2 * 1024)
        .unwrap();
    let want =
        &rasters()[0].tiles.iter().find(|t| (t.level, t.col, t.row) == (2, 1, 1)).unwrap().bytes.clone();
    assert!((want.len() as u64) < u64::from(chunk.stored_length) / 4, "a tile is a small part of its chunk");

    let mut host = Host::new(bytes);
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    let opened = host.served.len();
    let h = host.run(&mut rt, f, |rt| rt.raster_tile(f, 0, 2, 1, 1)).unwrap();
    assert_eq!(rt.raster_u8(h, 0).unwrap(), &want[..]);
    let asked: Vec<(u64, u32)> = host.served[opened..].to_vec();
    // placements (the components block), then the index block, then the tile: in turn, each its own range
    assert!(
        asked.iter().any(|&(o, l)| o == index_block.offset && l == index_block.stored_length),
        "{asked:?}"
    );
    let tile_ranges: Vec<&(u64, u32)> = asked
        .iter()
        .filter(|&&(o, _)| o >= chunk.offset && o < chunk.offset + u64::from(chunk.stored_length))
        .collect();
    assert_eq!(tile_ranges.len(), 1, "one range inside the chunk: {asked:?}");
    assert_eq!(tile_ranges[0].1 as usize, want.len(), "the tile's own length");
    assert!(asked.iter().all(|&(_, l)| l != chunk.stored_length), "the whole chunk was never asked for");

    // releasing the handle drops the tile's range, so the next call asks for it again
    rt.release(h).unwrap();
    assert_eq!(rt.raster_tile(f, 0, 2, 1, 1), Err(AbiError::NeedRange));
    assert_eq!(rt.needed(f).unwrap(), vec![*tile_ranges[0]]);
    // and a handle that is not released keeps it: another tile does not disturb it
    host.serve(&mut rt, f);
    let again = rt.raster_tile(f, 0, 2, 1, 1).unwrap();
    assert_eq!(rt.raster_u8(again, 0).unwrap(), &want[..]);
}

#[test]
fn issue_221_a_range_opened_plan_names_every_index_block_at_once() {
    let mut host = Host::new(file());
    let mut rt = Runtime::new();
    let f = host.open(&mut rt);
    let before = host.served.len();
    let mut rounds = 0;
    let h = loop {
        match rt.raster_plan(f, window(WHOLE), 1.0, 0.0) {
            Err(AbiError::NeedRange) => {
                rounds += 1;
                host.serve(&mut rt, f);
            }
            other => break other.unwrap(),
        }
    };
    assert!(rounds <= 2, "the placements, then both index blocks together: {rounds} rounds");
    let probe =
        ContainerReader::open(MemorySource::new(host.bytes.clone()), ReaderConfig::default()).unwrap();
    for cell in [0, 1] {
        let e = probe.entries().iter().find(|e| e.kind == KIND_INDEX && e.cell == cell).unwrap();
        assert!(
            host.served[before..]
                .iter()
                .any(|&(o, l)| o <= e.offset && e.offset + u64::from(e.stored_length) <= o + u64::from(l)),
            "the index block of raster {cell} was never provided"
        );
    }
    let tiles_asked = host.served[before..].iter().any(|&(o, _)| {
        probe
            .entries()
            .iter()
            .any(|e| e.kind == KIND_TILES && o >= e.offset && o < e.offset + u64::from(e.stored_length))
    });
    assert!(!tiles_asked, "a plan reads no tile");
    assert_eq!(rt.raster_u32(h, 0).unwrap(), &[0, 1]);
}

#[test]
fn issue_221_a_released_tile_handle_stays_refused_and_its_slot_is_reused_with_a_new_generation() {
    let (mut rt, f) = open_whole(file());
    let a = rt.raster_tile(f, 0, 2, 0, 0).unwrap();
    rt.release(a).unwrap();
    assert_eq!(rt.raster_u8(a, 0), Err(AbiError::Released));
    assert_eq!(rt.release(a), Err(AbiError::Released));
    let b = rt.raster_tile(f, 0, 2, 0, 0).unwrap();
    assert_ne!(a, b, "the slot is reused with the next generation");
    assert_eq!(rt.raster_u8(a, 0), Err(AbiError::Released), "the old handle is still refused");
    assert!(rt.raster_u8(b, 0).is_ok());
    assert!(b < 1 << 31, "a handle never reads as an error code");
}

#[test]
fn issue_221_no_raster_path_of_the_module_reads_a_whole_block() {
    let src = |p: &str| fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(p)).unwrap();
    for file in ["src/runtime.rs", "src/bindings.rs"] {
        assert!(!src(file).contains("read_stored"), "{file} reads through read_stored");
    }
    let facade = src("../pvec/src/unstable.rs");
    assert!(!facade[facade.find("raster_placements").unwrap()..].contains("read_stored"));
}
