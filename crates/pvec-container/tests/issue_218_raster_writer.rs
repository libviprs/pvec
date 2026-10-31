//! Issue #218 (R1.4): the writer for the raster blocks. It takes the encoded tiles with their place from the
//! producer and owns everything else: the order of the tile table, the chunking, which coarsest levels are
//! inline, the order of the blocks, and every number in the directory entries. The same tiles in any order give
//! the same bytes, and what a reader would refuse the writer refuses first.

mod support;

use pvec_components::{RASTER_CAPABILITY, RasterId, Stream, encode_rasters};
use pvec_container::blocks::raster::{CAPABILITY, KIND_INDEX, KIND_TILES, RasterIndex, TileIn, subtype};
use pvec_container::directory::BlockKind;
use pvec_container::limits::{LimitKind, Limits};
use pvec_container::reader::{ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, WriteError};
use support::art::Art;
use support::raster::*;

fn id(i: u32) -> RasterId {
    RasterId::new(i).unwrap()
}

fn open(bytes: Vec<u8>) -> ContainerReader<MemorySource<Vec<u8>>> {
    ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).expect("the file opens")
}

#[test]
fn two_rasters_are_written_and_every_tile_reads_back_equal() {
    let bytes = two_rasters_file();
    let reader = open(bytes);
    assert_eq!(reader.rasters(), vec![0, 1]);
    let ps = two_placements();
    for (r, p) in ps.iter().enumerate() {
        let skip = |l: u32, c: u32, row: u32| r == 0 && l == 2 && (c + row) % 2 == 1;
        for t in tiles(r as u32, p, 700, skip) {
            let got = reader.read_raster_tile(r as u32, t.level, t.col, t.row).expect("a tile reads");
            if t.bytes.is_empty() {
                assert!(got.is_none());
            } else {
                assert_eq!(
                    got.expect("stored").as_slice(),
                    &t.bytes[..],
                    "raster {r} {:?}",
                    (t.level, t.col, t.row)
                );
            }
        }
    }
    reader.validate().expect("the file validates");
    for r in 0..2 {
        reader.validate_raster(r).expect("each raster validates");
    }
}

#[test]
fn the_same_tiles_written_twice_give_identical_bytes() {
    assert_eq!(two_rasters_file(), two_rasters_file());
}

#[test]
fn a_different_arrival_order_of_tiles_and_of_rasters_gives_identical_bytes() {
    let reference = two_rasters_file();
    let ps = two_placements();
    let sparse = |l: u32, c: u32, r: u32| l == 2 && (c + r) % 2 == 1;
    for seed in 1..=12u64 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut shuffle = |v: &mut Vec<TileIn>| {
            for i in (1..v.len()).rev() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.swap(i, (x % (i as u64 + 1)) as usize);
            }
        };
        let mut a = tiles(0, &ps[0], 700, sparse);
        let mut b = tiles(1, &ps[1], 700, |_, _, _| false);
        shuffle(&mut a);
        shuffle(&mut b);
        let mut w = writer_with(&ps);
        // the second raster first, and each in two pushes
        let half = b.len() / 2;
        let tail = b.split_off(half);
        w.push_raster_tiles(id(1), tail).unwrap();
        w.push_raster_tiles(id(1), b).unwrap();
        w.push_raster_tiles(id(0), a).unwrap();
        assert_eq!(w.finish().unwrap(), reference, "seed {seed}");
    }
}

#[test]
fn kinds_9_and_10_sort_after_every_geometry_block_and_the_kind_9_blocks_are_adjacent() {
    let art = Art::new(two_rasters_file());
    let kinds: Vec<u16> = art.entries.iter().map(|e| e.kind).collect();
    let first_raster =
        kinds.iter().position(|k| *k == KIND_INDEX || *k == KIND_TILES).expect("raster blocks");
    assert!(kinds[..first_raster].iter().all(|k| BlockKind::from_id(*k).is_some()), "{kinds:?}");
    assert!(kinds[first_raster..].iter().all(|k| *k == KIND_INDEX || *k == KIND_TILES), "{kinds:?}");
    let nines: Vec<usize> = (0..kinds.len()).filter(|i| kinds[*i] == KIND_INDEX).collect();
    assert_eq!(nines.len(), 2);
    assert_eq!(nines[1], nines[0] + 1, "all kind 9 blocks are adjacent");
    assert_eq!(first_raster, nines[0], "and they come first");
    for e in art.entries.iter().filter(|e| e.kind == KIND_INDEX || e.kind == KIND_TILES) {
        assert_eq!(e.capability, CAPABILITY);
        assert_eq!(e.codec, 0, "stored as it is: a tile is read by range");
        assert!(e.flags & pvec_container::directory::FLAG_HAS_BOUNDS != 0);
        assert!(e.flags & pvec_container::directory::FLAG_MUST_UNDERSTAND == 0, "optional by construction");
    }
}

#[test]
fn layout_04_keys_are_unique_and_the_subtype_rule_ascends_by_level() {
    let art = Art::new(two_rasters_file());
    let keys: Vec<(u16, u32, u16, u16)> = art
        .entries
        .iter()
        .map(|e| (if e.kind == 3 { 2 } else { e.kind }, e.cell, e.kind, e.subtype))
        .collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "strictly ascending, so unique: {keys:?}");
    let level_of = |st: u16| u32::from(st) / 1024;
    for cell in 0..2 {
        let st: Vec<u16> = art
            .entries
            .iter()
            .filter(|e| e.kind == KIND_TILES && e.cell == cell)
            .map(|e| e.subtype)
            .collect();
        assert!(st.windows(2).all(|w| level_of(w[0]) <= level_of(w[1])), "raster {cell}: {st:?}");
        assert!(st.iter().all(|s| *s == subtype(level_of(*s), u32::from(*s) % 1024).unwrap()));
    }
}

#[test]
fn the_directory_entries_say_what_is_in_the_blocks() {
    let art = Art::new(two_rasters_file());
    let reader = open(art.bytes.clone());
    for e in art.entries.iter().filter(|e| e.kind == KIND_INDEX) {
        let index = reader.raster_index(e.cell).unwrap();
        assert_eq!(e.count, index.stored());
        // the footprint of the visible box: the image is upright at one unit a pixel
        let ps = two_placements();
        let p = &ps[e.cell as usize];
        let b = e.bounds.expect("the footprint");
        assert_eq!((b.min, b.max), ([0.0, 0.0], [f64::from(p.width), f64::from(p.height)]));
    }
    for e in art.entries.iter().filter(|e| e.kind == KIND_TILES) {
        let index = reader.raster_index(e.cell).unwrap();
        let (level, chunk) = (u32::from(e.subtype) / 1024, u32::from(e.subtype) % 1024);
        let in_chunk =
            index.level_tiles(level).unwrap().iter().filter(|t| t.is_stored() && t.chunk == chunk).count();
        assert_eq!(e.count as usize, in_chunk);
        let b = e.bounds.expect("the box of the chunk's tiles");
        assert!(b.min[0] < b.max[0] && b.min[1] < b.max[1]);
    }
}

#[test]
fn the_coarsest_levels_are_inline_up_to_the_cap_and_coarsest_first_across_rasters() {
    let ps = two_placements();
    let write = |cap: u64| {
        let mut w = writer_with(&ps);
        w.raster_inline_bytes(cap);
        w.push_raster_tiles(id(0), tiles(0, &ps[0], 700, |_, _, _| false)).unwrap();
        w.push_raster_tiles(id(1), tiles(1, &ps[1], 700, |_, _, _| false)).unwrap();
        w.finish().unwrap()
    };
    let inline_levels = |bytes: Vec<u8>, raster: u32| {
        let reader = open(bytes);
        let index = reader.raster_index(raster).unwrap();
        index.levels().iter().filter(|l| l.chunks == 0).count()
    };
    // level 0 of each raster is one tile of 700 bytes, level 1 of raster 0 is 2 by 2 of them
    assert_eq!(inline_levels(write(0), 0), 0);
    assert_eq!(inline_levels(write(1400), 0), 1);
    assert_eq!(inline_levels(write(1400), 1), 1);
    // 1400 for the two coarsest levels, then raster 0's second level (4 tiles) does not fit in 1500
    assert_eq!(inline_levels(write(1500), 0), 1);
    assert_eq!(
        inline_levels(write(1400 + 2800), 0),
        2,
        "raster 0 level 1 and raster 1 level 1 (4 tiles) compete for 2800"
    );
    // the default is 256 KiB and holds all of these
    let mut w = writer_with(&ps);
    w.push_raster_tiles(id(0), tiles(0, &ps[0], 700, |_, _, _| false)).unwrap();
    let all = open(w.finish().unwrap());
    let index = all.raster_index(0).unwrap();
    assert!(index.levels().iter().all(|l| l.chunks == 0), "12 + 4 + 1 tiles of 700 bytes are under 256 KiB");
    // and the same file is the same whichever order the budget was set in
    assert_eq!(write(1500), write(1500));
}

#[test]
fn a_level_over_the_chunk_limit_is_split_into_chunks_that_never_pass_it() {
    let ps = two_placements();
    let mut w = writer_with(&ps);
    w.raster_inline_bytes(0);
    w.raster_chunk_bytes(2000);
    w.push_raster_tiles(id(0), tiles(0, &ps[0], 700, |_, _, _| false)).unwrap();
    let art = Art::new(w.finish().unwrap());
    let level2: Vec<_> =
        art.entries.iter().filter(|e| e.kind == KIND_TILES && e.subtype / 1024 == 2).collect();
    assert_eq!(level2.len(), 6, "12 tiles, two to a chunk of at most 2000 bytes");
    assert!(level2.iter().all(|e| e.stored_length <= 2000 && e.count == 2));
    let reader = open(art.bytes.clone());
    reader.validate_raster(0).unwrap();
    let index: std::sync::Arc<RasterIndex> = reader.raster_index(0).unwrap();
    assert_eq!(index.levels()[2].chunks, 6);
}

#[test]
fn the_writer_refuses_what_the_reader_would() {
    let ps = two_placements();
    // tiles for a raster the stream does not place
    let mut w = writer_with(&ps);
    w.push_raster_tiles(id(7), vec![TileIn { level: 0, col: 0, row: 0, bytes: bytes_of(1, 0, 0, 0, 100) }])
        .unwrap();
    let e = w.finish().expect_err("no placement");
    assert!(matches!(e, WriteError::Raster(_)), "{e:?}");
    assert_eq!(e.rule(), "WRT-15");
    assert!(e.to_string().contains("RASTER-06"), "{e}");
    // tiles with no raster stream at all
    let mut bare = ContainerWriter::new(support::config(&[], None));
    bare.push_line(
        pvec_container::writer::EntityMeta {
            id: support::eid(0),
            style: 0,
            cell: 0,
            bounds: pvec_container::Bounds { min: [0.0, 0.0], max: [1.0, 1.0] },
        },
        [0.0, 0.0],
        [1.0, 1.0],
    )
    .unwrap();
    bare.push_raster_tiles(
        id(0),
        vec![TileIn { level: 0, col: 0, row: 0, bytes: bytes_of(1, 0, 0, 0, 100) }],
    )
    .unwrap();
    assert!(matches!(bare.finish(), Err(WriteError::Raster(_))));
    // a tile outside the grid, and the same tile twice
    for bad in [
        vec![TileIn { level: 0, col: 1, row: 0, bytes: bytes_of(1, 0, 0, 0, 100) }],
        vec![TileIn { level: 9, col: 0, row: 0, bytes: bytes_of(1, 0, 0, 0, 100) }],
        vec![
            TileIn { level: 0, col: 0, row: 0, bytes: bytes_of(1, 0, 0, 0, 100) },
            TileIn { level: 0, col: 0, row: 0, bytes: bytes_of(1, 0, 0, 0, 101) },
        ],
    ] {
        let mut w = writer_with(&ps);
        w.push_raster_tiles(id(1), bad).unwrap();
        let e = w.finish().expect_err("refused at write time");
        assert!(matches!(e, WriteError::Raster(_)), "{e:?}");
    }
    // a tile longer than a chunk may be
    let mut w = writer_with(&ps);
    w.raster_inline_bytes(0);
    w.raster_chunk_bytes(100);
    w.push_raster_tiles(id(1), tiles(1, &ps[1], 700, |_, _, _| false)).unwrap();
    assert!(matches!(w.finish(), Err(WriteError::Raster(_))));
}

#[test]
fn a_placement_with_no_tiles_is_a_file_with_no_raster_blocks_for_it() {
    let ps = two_placements();
    let mut w = writer_with(&ps);
    w.push_raster_tiles(id(1), tiles(1, &ps[1], 700, |_, _, _| false)).unwrap();
    let reader = open(w.finish().unwrap());
    assert_eq!(reader.rasters(), vec![1]);
    let e = reader.raster_index(0).expect_err("nothing was written for it");
    assert_eq!(e.rule(), "RASTER-06");
}

#[test]
fn limit_14_a_raster_of_131073_tiles_is_refused_under_the_browser_preset_and_read_under_the_default() {
    // 81,920 by 81,920 pixels in tiles of 256: 320 by 320 tiles at full size and 136,539 over the pyramid, none
    // stored, so the file is a table and no pixel
    let p = placement(81_920, 81_920, 256);
    let total: u64 = grid(&p).iter().map(|(c, r)| u64::from(*c) * u64::from(*r)).sum();
    assert!((131_073..=1 << 20).contains(&total), "{total}");
    let mut w = writer_with(std::slice::from_ref(&p));
    w.push_raster_tiles(id(0), Vec::new()).unwrap();
    let bytes = w.finish().unwrap();
    let browser = ContainerReader::open(
        MemorySource::new(bytes.clone()),
        ReaderConfig { limits: Limits::BROWSER, ..ReaderConfig::default() },
    )
    .unwrap();
    let e = browser.raster_index(0).expect_err("over the browser ceiling");
    assert_eq!(e.rule(), "LIMIT-14");
    assert!(
        matches!(&e, ReadError::Limit(l) if l.limit == LimitKind::RasterTiles && l.declared == total),
        "{e:?}"
    );
    let default = open(bytes);
    assert_eq!(default.raster_index(0).unwrap().tiles().len() as u64, total);
}

#[test]
fn the_stream_is_the_placement_the_blocks_are_built_from() {
    // the grid of the index is the grid of the placement in the stream, whatever the writer was handed
    let ps = two_placements();
    let reader = open(two_rasters_file());
    for (r, p) in ps.iter().enumerate() {
        let index = reader.raster_index(r as u32).unwrap();
        index.check_grid(p.width, p.height, u32::from(p.tile_edge)).unwrap();
        assert_eq!(index.levels().len(), usize::from(p.levels));
    }
    let _ = (
        RASTER_CAPABILITY,
        Stream {
            capability: RASTER_CAPABILITY,
            version: pvec_capabilities::Version::new(1, 0, 0),
            payload: encode_rasters(&[]).unwrap(),
        },
    );
}

#[test]
fn a_reader_that_does_not_implement_21_skips_real_raster_blocks_and_the_geometry_is_bit_identical() {
    // the third independence case of ADR 0018, now with real block content: the geometry of the file with the
    // rasters, read by a reader without capability 21, is the geometry of the file without them
    use pvec_container::directory::BlockKind;
    use pvec_container::negotiate::{Effect, ReaderCapabilities, SkipReason};
    use pvec_container::reader::BlockId;
    let ps = two_placements();
    let with = {
        let mut w = writer_with(&ps);
        w.raster_inline_bytes(0);
        w.raster_chunk_bytes(2000);
        w.push_raster_tiles(id(0), tiles(0, &ps[0], 700, |_, _, _| false)).unwrap();
        w.push_raster_tiles(id(1), tiles(1, &ps[1], 700, |_, _, _| false)).unwrap();
        w.finish().unwrap()
    };
    let without = writer_with(&ps).finish().unwrap();
    let old = |bytes: Vec<u8>| {
        let cfg = ReaderConfig {
            capabilities: ReaderCapabilities::v1().without(pvec_capabilities::CapabilityId::new(21)),
            ..ReaderConfig::default()
        };
        ContainerReader::open(MemorySource::new(bytes), cfg)
            .expect("an optional capability does not stop the open")
    };
    let (a, b) = (old(with), old(without));
    let partial = a.partial_open().expect("a degraded open");
    let c = partial.capabilities.iter().find(|c| c.id.get() == 21).expect("21 is listed");
    assert_eq!((c.reason, c.effect), (SkipReason::NotImplemented, Effect::Unrastered));
    let raster_blocks = a.entries().iter().filter(|e| e.kind == KIND_INDEX || e.kind == KIND_TILES).count();
    assert!(raster_blocks > 4);
    assert_eq!(partial.blocks.len(), raster_blocks, "every raster block is a skip naming 21");
    assert!(partial.blocks.iter().all(|s| s.reason == SkipReason::UnknownBlockKind));
    a.validate().expect("the file validates around the blocks it skips");
    let geometry = |r: &ContainerReader<MemorySource<Vec<u8>>>| -> Vec<Vec<u8>> {
        (0..r.entries().len() as u32)
            .filter(|i| {
                matches!(
                    r.entries()[*i as usize].block_kind(),
                    Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)
                )
            })
            .map(|i| r.read_block(BlockId(i)).unwrap().as_bytes().to_vec())
            .collect()
    };
    assert_eq!(geometry(&a), geometry(&b));
    let first = a.entries().iter().position(|e| e.kind == KIND_INDEX).unwrap();
    assert!(matches!(a.read_block(BlockId(first as u32)), Err(ReadError::Skipped { .. })));
}
