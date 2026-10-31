//! Issue #218 (R1.4): the raster index and tile blocks in a file, read back through the reader. Files are laid
//! out by the assembler from the blocks `blocks::raster::build` makes, so a hostile one is the same blocks with
//! a byte changed. A tile read is a range inside its chunk, checked against the index's XXH3-64 before the bytes
//! leave the reader, through the reader's accounting and never through `read_stored`.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_components::Placement;
use pvec_container::Bounds;
use pvec_container::blocks::raster::{
    CAPABILITY, IndexError, KIND_INDEX, KIND_TILES, RasterIndex, TileIn, build, subtype,
};
use pvec_container::directory::FLAG_HAS_BOUNDS;
use pvec_container::integrity::{Integrity, block_checksum};
use pvec_container::reader::{AsyncContainerReader, ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::RawBlock;
use std::future::Future;
use support::art::{Art, rebuild};
use support::raster::{grid, placement, writer_with};

const MAX: u64 = 1 << 20;

/// One raster's tiles and how they are laid out.
struct R {
    id: u32,
    grid: Vec<(u32, u32)>,
    tiles: Vec<TileIn>,
    inline: u32,
    chunk_bytes: u32,
}

use support::raster::bytes_of;

/// The bytes of a tile, and how many a chunk may hold: two tiles to a chunk.
const LEN: usize = 100;
const CHUNK: u32 = 250;

fn pyramid(id: u32, grid: &[(u32, u32)], len: usize, skip: impl Fn(u32, u32, u32) -> bool) -> Vec<TileIn> {
    let mut out = Vec::new();
    for (l, &(cols, rows)) in grid.iter().enumerate() {
        for row in 0..rows {
            for col in 0..cols {
                let bytes =
                    if skip(l as u32, col, row) { Vec::new() } else { bytes_of(id, l as u32, col, row, len) };
                out.push(TileIn { level: l as u32, col, row, bytes });
            }
        }
    }
    out
}

/// The placements of the two rasters: 0 is 1000 by 600 (three levels, 4 by 3 at full size) and 1 is 1400 by 900 (four
/// levels, 6 by 4 at full size).
fn placements() -> Vec<Placement> {
    vec![placement(1000, 600, 256), placement(1400, 900, 256)]
}

/// Two rasters: 0 is sparse with its coarsest level inline, 1 has a level split into chunks.
fn two() -> Vec<R> {
    let ps = placements();
    let (g0, g1) = (grid(&ps[0]), grid(&ps[1]));
    vec![
        R {
            id: 0,
            tiles: pyramid(0, &g0, LEN, |l, c, r| l == 2 && (c + r) % 2 == 1),
            grid: g0,
            inline: 1,
            chunk_bytes: 1 << 20,
        },
        R { id: 1, tiles: pyramid(1, &g1, LEN, |_, _, _| false), grid: g1, inline: 0, chunk_bytes: CHUNK },
    ]
}

fn blocks_of(r: &R) -> Vec<RawBlock> {
    let built = build(&r.grid, r.tiles.clone(), r.inline, r.chunk_bytes, MAX).expect("the tiles build");
    let footprint = Some(Bounds { min: [0.0, 0.0], max: [10.0, 6.0] });
    let mut out = vec![block(KIND_INDEX, r.id, 0, built.index.stored(), built.index.encode(), footprint)];
    for c in built.chunks {
        let st = subtype(c.level, c.chunk).expect("a subtype");
        out.push(block(KIND_TILES, r.id, st, c.count, c.bytes, footprint));
    }
    out
}

fn block(
    kind: u16,
    cell: u32,
    subtype: u16,
    count: u32,
    stored: Vec<u8>,
    bounds: Option<Bounds>,
) -> RawBlock {
    RawBlock {
        kind,
        subtype,
        cell,
        count,
        bounds,
        flags: FLAG_HAS_BOUNDS,
        codec: 0,
        capability: CAPABILITY,
        uncompressed_length: stored.len() as u32,
        stored,
    }
}

/// A drawing with one line and the raster stream for the two placements, and no raster block yet.
fn base() -> Art {
    Art::new(writer_with(&placements()).finish().unwrap())
}

/// `base` (which declares capability 21 with its stream) with `raster_blocks` after the geometry, in the canonical order:
/// every kind 9 block by raster, then every kind 10 block by raster and subtype.
fn with_blocks(base: &Art, integrity: Integrity, mut raster_blocks: Vec<RawBlock>) -> Art {
    raster_blocks.sort_by_key(|b| (b.kind, b.cell, b.subtype));
    rebuild(base, integrity, |_, blocks| blocks.extend(raster_blocks))
}

fn file(integrity: Integrity) -> (Art, Vec<R>) {
    let rasters = two();
    let blocks = rasters.iter().flat_map(blocks_of).collect();
    (with_blocks(&base(), integrity, blocks), rasters)
}

type Reader = ContainerReader<Recording<MemorySource<Vec<u8>>>>;

fn open(art: &Art) -> Reader {
    ContainerReader::open(Recording::new(MemorySource::new(art.bytes.clone())), ReaderConfig::default())
        .expect("the file opens")
}

fn block_on<F: Future>(f: F) -> F::Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn entry_of(art: &Art, kind: u16, cell: u32, st: u16) -> usize {
    art.entries.iter().position(|e| e.kind == kind && e.cell == cell && e.subtype == st).expect("the entry")
}

#[test]
fn every_tile_of_two_rasters_reads_back_equal_by_digest() {
    let (art, rasters) = file(Integrity::Full);
    let reader = open(&art);
    assert_eq!(reader.rasters(), vec![0, 1]);
    for r in &rasters {
        for t in &r.tiles {
            let got = reader.read_raster_tile(r.id, t.level, t.col, t.row).expect("a tile reads");
            if t.bytes.is_empty() {
                assert!(got.is_none(), "raster {} tile {:?} is not stored", r.id, (t.level, t.col, t.row));
            } else {
                let got = got.expect("a stored tile has bytes");
                assert_eq!(block_checksum(&got), block_checksum(&t.bytes));
                assert_eq!(got.as_slice(), &t.bytes[..]);
            }
        }
    }
    reader.validate().expect("the file validates with the raster blocks in it");
    for r in &rasters {
        reader.validate_raster(r.id).expect("every raster validates");
    }
}

#[test]
fn a_tile_that_is_not_stored_returns_no_bytes_and_no_error() {
    let (art, _) = file(Integrity::Full);
    let reader = open(&art);
    // (c + r) odd at level 2 of raster 0 is not stored
    assert_eq!(reader.read_raster_tile(0, 2, 1, 0).expect("not an error"), None);
    assert_eq!(reader.read_raster_tile(0, 2, 0, 1).expect("not an error"), None);
    assert!(reader.read_raster_tile(0, 2, 0, 0).unwrap().is_some());
}

#[test]
fn a_tile_outside_the_pyramid_is_an_error_for_the_caller() {
    let (art, _) = file(Integrity::Full);
    let reader = open(&art);
    for (l, c, r) in [(3, 0, 0), (2, 4, 0), (2, 0, 3), (0, 1, 0)] {
        let e = reader.read_raster_tile(0, l, c, r).expect_err("outside the pyramid");
        assert!(matches!(e, ReadError::Raster(IndexError::Tile { .. })), "{e:?}");
    }
    let e = reader.read_raster_tile(9, 0, 0, 0).expect_err("no such raster");
    assert!(matches!(e, ReadError::Raster(IndexError::NoPlacement { raster: 9 })), "{e:?}");
    assert_eq!(e.rule(), "RASTER-06");
}

#[test]
fn the_index_is_read_lazily_and_a_tile_is_one_range_inside_its_chunk() {
    let (art, _) = file(Integrity::Full);
    let reader = open(&art);
    let index_entry = &art.entries[entry_of(&art, KIND_INDEX, 1, 0)];
    // opening reads the first 4 KiB and the directory, which in a file this small can overlap a block, so the
    // question is whether the index was asked for as a block
    let asked = |e: &pvec_container::directory::BlockEntry| {
        reader.source().requests().iter().any(|r| (r.offset, r.len) == (e.offset, e.stored_length))
    };
    assert!(!asked(index_entry), "opening does not read an index");
    let before = reader.source().requests().len();
    // level 2 of raster 1 is 3 by 2 tiles of 40 bytes, 100 a chunk, so two to a chunk
    let got = reader.read_raster_tile(1, 2, 0, 0).unwrap().expect("stored");
    assert_eq!(got.as_slice(), &bytes_of(1, 2, 0, 0, LEN)[..]);
    let seen = reader.source().requests();
    assert_eq!(
        seen.len() - before,
        3,
        "the component set, the index block and the tile: {:?}",
        &seen[before..]
    );
    let components = art.entries.iter().find(|e| e.kind == 4).expect("the component set");
    assert_eq!((seen[before].offset, seen[before].len), (components.offset, components.stored_length));
    assert_eq!(
        (seen[before + 1].offset, seen[before + 1].len),
        (index_entry.offset, index_entry.stored_length)
    );
    let tile = seen[before + 2];
    assert_eq!(tile.len as usize, LEN, "the range of the tile and not of its chunk");
    // the request lies inside a chunk block of raster 1, level 2, and that chunk is longer than the request
    let chunk = art
        .entries
        .iter()
        .find(|e| {
            e.kind == KIND_TILES
                && e.cell == 1
                && e.offset <= tile.offset
                && tile.offset + 40 <= e.offset + u64::from(e.stored_length)
        })
        .expect("inside a chunk");
    assert!(chunk.stored_length as usize > LEN);
    // the second read of the same raster does not read the index again
    reader.read_raster_tile(1, 2, 1, 0).unwrap();
    assert_eq!(reader.source().requests().len() - before, 4, "one more range, the tile");
}

#[test]
fn no_tile_read_ever_asks_for_a_whole_chunk_so_read_stored_is_not_on_the_path() {
    let (art, rasters) = file(Integrity::Full);
    let reader = open(&art);
    for r in &rasters {
        for t in &r.tiles {
            reader.read_raster_tile(r.id, t.level, t.col, t.row).unwrap();
        }
    }
    let chunks: Vec<_> = art.entries.iter().filter(|e| e.kind == KIND_TILES && e.count > 1).collect();
    assert!(!chunks.is_empty());
    for req in reader.source().requests() {
        for c in &chunks {
            assert!(
                !(req.offset == c.offset && req.len == c.stored_length),
                "a request for the whole of a chunk: {req:?}"
            );
        }
    }
}

#[test]
fn an_inline_tile_costs_the_index_block_and_nothing_else() {
    let (art, _) = file(Integrity::Full);
    let reader = open(&art);
    reader.read_raster_tile(0, 0, 0, 0).unwrap().expect("the one tile of the coarsest level");
    let seen = reader.source().requests();
    let index_entry = &art.entries[entry_of(&art, KIND_INDEX, 0, 0)];
    assert_eq!(seen.last().map(|r| (r.offset, r.len)), Some((index_entry.offset, index_entry.stored_length)));
}

#[test]
fn a_flipped_byte_inside_a_tile_fails_the_tile_check_and_the_file_still_opens() {
    let (art, rasters) = file(Integrity::Full);
    // a byte of the tile (3, 0, 0) of raster 1
    let tile = rasters[1].tiles.iter().find(|t| (t.level, t.col, t.row) == (3, 0, 0)).unwrap();
    let index = RasterIndex::parse(
        &art.bytes[art.entries[entry_of(&art, KIND_INDEX, 1, 0)].offset as usize..]
            [..art.entries[entry_of(&art, KIND_INDEX, 1, 0)].stored_length as usize],
        MAX,
    )
    .unwrap();
    let t = index.tile(3, 0, 0).unwrap();
    let chunk = art.entries[entry_of(&art, KIND_TILES, 1, subtype(3, t.chunk).unwrap())];
    let at = chunk.offset as usize + t.offset as usize + 5;
    let mut bytes = art.bytes.clone();
    bytes[at] ^= 0x10;
    assert_ne!(bytes[at], art.bytes[at]);
    let reader = ContainerReader::open(
        MemorySource::new(bytes),
        ReaderConfig { require_integrity: false, ..ReaderConfig::default() },
    )
    .expect("the file opens: the flip is in a payload");
    let e = reader.read_raster_tile(1, 3, 0, 0).expect_err("the tile check fails");
    assert_eq!(e.rule(), "INTEGRITY-05", "{e}");
    // the neighbours in the same chunk, the other chunks and the geometry are fine
    assert!(reader.read_raster_tile(1, 3, 1, 0).is_ok());
    assert!(reader.read_raster_tile(1, 1, 0, 0).is_ok());
    let _ = tile;
    // full validation fails the chunk, by the whole-block checksum
    let e = reader.validate().expect_err("full validation");
    assert_eq!(e.rule(), "INTEGRITY-03", "{e}");
    let e = reader.validate_raster(1).expect_err("the raster does not validate");
    assert!(matches!(e.rule().as_str(), "INTEGRITY-03" | "INTEGRITY-05"), "{e}");
}

#[test]
fn the_tile_check_holds_in_a_file_with_no_checksum_table() {
    let (art, _) = file(Integrity::None);
    let index = RasterIndex::parse(
        &art.bytes[art.entries[entry_of(&art, KIND_INDEX, 1, 0)].offset as usize..]
            [..art.entries[entry_of(&art, KIND_INDEX, 1, 0)].stored_length as usize],
        MAX,
    )
    .unwrap();
    let t = *index.tile(1, 1, 0).unwrap();
    let chunk = art.entries[entry_of(&art, KIND_TILES, 1, subtype(1, t.chunk).unwrap())];
    let mut bytes = art.bytes.clone();
    bytes[chunk.offset as usize + t.offset as usize] ^= 1;
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    assert_eq!(reader.read_raster_tile(1, 1, 1, 0).expect_err("checked").rule(), "INTEGRITY-05");
}

#[test]
fn the_async_reader_reads_the_same_tiles() {
    let (art, rasters) = file(Integrity::Full);
    let reader =
        block_on(AsyncContainerReader::open(MemorySource::new(art.bytes.clone()), ReaderConfig::default()))
            .unwrap();
    for t in &rasters[1].tiles {
        let got = block_on(reader.read_raster_tile(1, t.level, t.col, t.row)).unwrap().expect("stored");
        assert_eq!(got.as_slice(), &t.bytes[..]);
    }
    block_on(reader.validate_raster(1)).unwrap();
}

// RASTER-02 and RASTER-06 in a file: each refuses the raster and not the file

fn refused_raster(art: &Art, raster: u32) -> ReadError {
    let reader = open(art);
    reader.validate().ok();
    reader.raster_index(raster).expect_err("the raster is refused")
}

fn edit(raster_blocks: Vec<RawBlock>, f: impl FnOnce(&mut Vec<RawBlock>)) -> Vec<RawBlock> {
    let mut b = raster_blocks;
    f(&mut b);
    b
}

#[test]
fn raster_02_an_index_that_lies_about_its_size_is_refused_and_the_other_raster_still_reads() {
    let rasters = two();
    let mut all: Vec<RawBlock> = rasters.iter().flat_map(blocks_of).collect();
    // raster 0's index: the header claims one more tile than the table holds (the offset of tile_total)
    let at = all.iter().position(|b| b.kind == KIND_INDEX && b.cell == 0).unwrap();
    let total = u32::from_le_bytes(all[at].stored[8..12].try_into().unwrap());
    all[at].stored[8..12].copy_from_slice(&(total + 1).to_le_bytes());
    let art = with_blocks(&base(), Integrity::Full, all);
    let reader = open(&art);
    let e = reader.raster_index(0).expect_err("refused");
    assert_eq!(e.rule(), "RASTER-02", "{e}");
    assert!(reader.read_raster_tile(1, 0, 0, 0).is_ok(), "the file and the other raster are fine");
    assert!(reader.validate().is_ok(), "the raster is refused, not the file");
}

#[test]
fn raster_02_a_chunk_block_that_is_too_short_too_long_or_missing_is_refused() {
    let rasters = two();
    let blocks: Vec<RawBlock> = rasters.iter().flat_map(blocks_of).collect();
    let st = subtype(3, 3).unwrap();
    let find = |b: &Vec<RawBlock>| {
        b.iter().position(|x| x.kind == KIND_TILES && x.cell == 1 && x.subtype == st).unwrap()
    };
    for (name, f) in [
        (
            "short",
            Box::new(|b: &mut Vec<RawBlock>| {
                let i = find(b);
                b[i].stored.pop();
                b[i].uncompressed_length -= 1;
            }) as Box<dyn Fn(&mut Vec<RawBlock>)>,
        ),
        (
            "long",
            Box::new(|b: &mut Vec<RawBlock>| {
                let i = find(b);
                b[i].stored.push(0);
                b[i].uncompressed_length += 1;
            }),
        ),
        (
            "missing",
            Box::new(|b: &mut Vec<RawBlock>| {
                let i = find(b);
                b.remove(i);
            }),
        ),
    ] {
        let art = with_blocks(&base(), Integrity::Full, edit(blocks.clone(), |b| f(b)));
        let e = refused_raster(&art, 1);
        assert_eq!(e.rule(), "RASTER-02", "{name}: {e}");
    }
}

#[test]
fn raster_06_a_tile_block_with_no_index_is_refused() {
    let rasters = two();
    let blocks = edit(rasters.iter().flat_map(blocks_of).collect(), |b| {
        b.retain(|x| !(x.kind == KIND_INDEX && x.cell == 1))
    });
    let art = with_blocks(&base(), Integrity::Full, blocks);
    let reader = open(&art);
    assert_eq!(reader.rasters(), vec![0, 1], "raster 1 is named by its tile blocks");
    let e = reader.raster_index(1).expect_err("no index");
    assert!(matches!(e, ReadError::Raster(IndexError::NoIndex { raster: 1 })), "{e:?}");
    assert_eq!(e.rule(), "RASTER-06");
    assert!(reader.raster_index(0).is_ok());
}

#[test]
fn raster_06_a_tile_block_of_a_level_or_chunk_the_index_does_not_have_is_refused() {
    let rasters = two();
    let blocks: Vec<RawBlock> = rasters.iter().flat_map(blocks_of).collect();
    for (name, level, chunk) in [("a level beyond levels", 5u32, 0u32), ("a chunk beyond the level's", 2, 20)]
    {
        let mut extra = blocks.clone();
        extra.push(block(KIND_TILES, 1, subtype(level, chunk).unwrap(), 1, vec![1, 2, 3], None));
        let art = with_blocks(&base(), Integrity::Full, extra);
        let e = refused_raster(&art, 1);
        assert!(matches!(e, ReadError::Raster(IndexError::Block { raster: 1, .. })), "{name}: {e:?}");
        assert_eq!(e.rule(), "RASTER-06");
    }
}

#[test]
fn raster_06_a_tile_block_stored_with_a_codec_is_refused_because_a_range_needs_it_as_it_is() {
    let rasters = two();
    let blocks = edit(rasters.iter().flat_map(blocks_of).collect(), |b| {
        let i = b.iter().position(|x| x.kind == KIND_TILES && x.cell == 1).unwrap();
        b[i].codec = 1; // lz4, a codec the reader knows
    });
    let art = with_blocks(&base(), Integrity::Full, blocks);
    let e = refused_raster(&art, 1);
    assert!(matches!(e, ReadError::Raster(IndexError::Block { raster: 1, .. })), "{e:?}");
}

#[test]
fn raster_06_a_count_in_the_directory_that_is_not_the_tiles_is_refused() {
    let rasters = two();
    let blocks = edit(rasters.iter().flat_map(blocks_of).collect(), |b| {
        let i = b.iter().position(|x| x.kind == KIND_INDEX && x.cell == 1).unwrap();
        b[i].count += 1;
    });
    let art = with_blocks(&base(), Integrity::Full, blocks);
    let e = refused_raster(&art, 1);
    assert!(matches!(e, ReadError::Raster(IndexError::Block { raster: 1, subtype: 0 })), "{e:?}");
}

#[test]
fn raster_06_a_kind_9_or_10_entry_with_another_capability_or_none_refuses_the_file() {
    let rasters = two();
    // 0 names nothing, 22 is a capability nobody registered, and 14 owns no kinds
    let base = base();
    // 14 is a component capability, which owns no block kinds
    for capability in [0u16, 22, 14] {
        let mut blocks = edit(rasters.iter().flat_map(blocks_of).collect(), |b| b[0].capability = capability);
        blocks.sort_by_key(|b| (b.kind, b.cell, b.subtype));
        let art = rebuild(&base, Integrity::Full, |m, existing| {
            for id in [14u32, 22] {
                m.capabilities.push(Declared {
                    id: CapabilityId::new(id),
                    requirement: Requirement::Optional,
                    version: Version::new(1, 0, 0),
                });
            }
            m.capabilities.sort_by_key(|d| d.id);
            existing.extend(blocks);
        });
        let e = ContainerReader::open(MemorySource::new(art.bytes), ReaderConfig::default())
            .err()
            .expect("the file is refused");
        assert_eq!(e.rule(), "NEGOTIATE-06", "capability {capability}: {e}");
    }
}

#[test]
fn the_default_ceiling_admits_the_index_of_a_pyramid_and_refuses_one_over_it() {
    // LIMIT-14 on the entries of one tile table: more than 2^20 is refused before the table is sized
    let mut header = vec![0u8; 32];
    header[..4].copy_from_slice(b"PVX1");
    header[4] = 1;
    header[6] = 1;
    header[8..12].copy_from_slice(&((1u32 << 20) + 1).to_le_bytes());
    let blocks = vec![block(KIND_INDEX, 0, 0, 0, header, None)];
    let art = with_blocks(&base(), Integrity::Full, blocks);
    let e = open(&art).raster_index(0).expect_err("over the ceiling");
    assert_eq!(e.rule(), "LIMIT-14", "{e}");
}
