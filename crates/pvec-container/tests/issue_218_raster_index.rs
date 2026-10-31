//! Issue #218 (R1.4): the raster index block and the tile blocks it points into, as bytes. `RASTER-02` is
//! checked before every allocation it protects, a parse is the inverse of `encode`, and the writer owns the
//! order: the tiles can arrive in any order and the blocks do not change.

use pvec_container::blocks::raster::{
    CHUNKS_PER_LEVEL, HEADER_LEN, INLINE, IndexError, LEVEL_LEN, RasterIndex, TILE_LEN, TileIn, build,
    grid_of, hilbert_key, subtype,
};

/// Distinct, recognisable bytes for a tile: its place, repeated.
fn bytes_of(level: u32, col: u32, row: u32, len: usize) -> Vec<u8> {
    (0..len).map(|i| (level * 31 + col * 7 + row * 3 + i as u32) as u8).collect()
}

/// Every tile of `grid`, each `len` bytes, except those `skip` names.
fn tiles(grid: &[(u32, u32)], len: usize, skip: impl Fn(u32, u32, u32) -> bool) -> Vec<TileIn> {
    let mut out = Vec::new();
    for (l, &(cols, rows)) in grid.iter().enumerate() {
        for row in 0..rows {
            for col in 0..cols {
                let bytes =
                    if skip(l as u32, col, row) { Vec::new() } else { bytes_of(l as u32, col, row, len) };
                out.push(TileIn { level: l as u32, col, row, bytes });
            }
        }
    }
    out
}

const GRID: [(u32, u32); 3] = [(1, 1), (2, 2), (5, 3)];
const MAX: u64 = 1 << 20;

fn small() -> Vec<u8> {
    build(&GRID, tiles(&GRID, 40, |_, _, _| false), 1, 1 << 20, MAX).unwrap().index.encode()
}

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// The header as the layout has it: the magic, the version, `levels` and `tiles`, with the rest zero.
fn header(levels: u16, tiles: u32) -> Vec<u8> {
    let mut b = vec![0u8; HEADER_LEN];
    b[..4].copy_from_slice(b"PVX1");
    put16(&mut b, 4, 1);
    put16(&mut b, 6, levels);
    put(&mut b, 8, tiles);
    b
}

fn refused(bytes: &[u8]) -> IndexError {
    RasterIndex::parse(bytes, MAX).expect_err("the block must be refused")
}

#[test]
fn the_hilbert_curve_visits_a_square_by_neighbours() {
    // the order of a 2 by 2, the first step along x: (0,0) (1,0) (1,1) (0,1)
    let key = |c, r| hilbert_key(2, c, r);
    assert_eq!([key(0, 0), key(1, 0), key(1, 1), key(0, 1)], [0, 1, 2, 3]);
    for side in [1u64, 2, 4, 8, 16] {
        let mut by_key = vec![(u32::MAX, u32::MAX); (side * side) as usize];
        for r in 0..side as u32 {
            for c in 0..side as u32 {
                let k = hilbert_key(side, c, r) as usize;
                assert_eq!(by_key[k], (u32::MAX, u32::MAX), "side {side}: key {k} twice");
                by_key[k] = (c, r);
            }
        }
        // a permutation of the square in which consecutive keys are neighbours
        for w in by_key.windows(2) {
            let d = w[0].0.abs_diff(w[1].0) + w[0].1.abs_diff(w[1].1);
            assert_eq!(d, 1, "side {side}: {:?} then {:?}", w[0], w[1]);
        }
    }
}

#[test]
fn the_subtype_is_level_times_1024_plus_chunk_and_ascends_by_level() {
    assert_eq!(subtype(0, 0), Some(0));
    assert_eq!(subtype(3, 5), Some(3 * 1024 + 5));
    assert_eq!(subtype(63, 1023), Some(u16::MAX));
    assert_eq!(subtype(64, 0), None);
    assert_eq!(subtype(0, CHUNKS_PER_LEVEL), None);
    let keys: Vec<u16> = (0..4).flat_map(|l| (0..3).map(move |c| subtype(l, c).unwrap())).collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "unique and ascending");
}

#[test]
fn an_index_parses_back_to_what_was_built_and_encodes_to_the_same_bytes() {
    let built = build(&GRID, tiles(&GRID, 40, |_, _, _| false), 1, 1 << 20, MAX).unwrap();
    let bytes = built.index.encode();
    let parsed = RasterIndex::parse(&bytes, MAX).unwrap();
    assert_eq!(parsed, built.index);
    assert_eq!(parsed.encode(), bytes);
    assert_eq!(parsed.levels().len(), 3);
    assert_eq!(parsed.tiles().len(), 1 + 4 + 15);
    assert_eq!(parsed.stored(), 20);
    for (l, &(cols, rows)) in GRID.iter().enumerate() {
        let level = &parsed.levels()[l];
        assert_eq!((level.cols, level.rows), (cols, rows));
        for row in 0..rows {
            for col in 0..cols {
                let t = parsed.tile(l as u32, col, row).expect("every tile of the grid has an entry");
                assert_eq!((t.col, t.row), (col, row));
                assert_eq!(t.length, 40);
            }
        }
    }
    assert!(parsed.tile(0, 1, 0).is_none() && parsed.tile(3, 0, 0).is_none(), "outside the pyramid");
}

#[test]
fn every_stored_tile_comes_back_equal_through_its_entry() {
    let given = tiles(&GRID, 40, |_, _, _| false);
    let built = build(&GRID, given.clone(), 1, 100, MAX).unwrap();
    let index = RasterIndex::parse(&built.index.encode(), MAX).unwrap();
    for t in given {
        let e = index.tile(t.level, t.col, t.row).unwrap();
        assert_eq!(e.xxh3, pvec_container::integrity::block_checksum(&t.bytes));
        let got: Vec<u8> = if e.chunk == INLINE {
            index.inline_bytes(e).unwrap().to_vec()
        } else {
            let c = built.chunks.iter().find(|c| c.level == t.level && c.chunk == e.chunk).unwrap();
            c.bytes[e.offset as usize..(e.offset + e.length) as usize].to_vec()
        };
        assert_eq!(got, t.bytes, "tile {:?}", (t.level, t.col, t.row));
    }
}

#[test]
fn the_arrival_order_of_the_tiles_does_not_change_a_byte() {
    let given = tiles(&GRID, 40, |l, c, r| (l + c + r) % 4 == 0);
    let a = build(&GRID, given.clone(), 1, 100, MAX).unwrap();
    let mut shuffled = given;
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for i in (1..shuffled.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        shuffled.swap(i, (x % (i as u64 + 1)) as usize);
    }
    assert_eq!(build(&GRID, shuffled, 1, 100, MAX).unwrap(), a);
}

#[test]
fn a_tile_with_no_bytes_is_not_stored_and_costs_nothing_but_its_entry() {
    let built =
        build(&GRID, tiles(&GRID, 40, |l, c, r| l == 2 && (c + r) % 2 == 1), 0, 1 << 20, MAX).unwrap();
    let index = RasterIndex::parse(&built.index.encode(), MAX).unwrap();
    let sparse = index.tile(2, 1, 0).unwrap();
    assert_eq!((sparse.length, sparse.xxh3, sparse.offset, sparse.chunk), (0, 0, 0, 0));
    assert!(!sparse.is_stored());
    assert_eq!(index.tiles().len(), 20, "an entry for every tile of the grid");
    // level 2 is 5 by 3: seven of its fifteen tiles sit where column plus row is odd
    assert_eq!(index.stored(), 20 - 7);
    let in_chunks: u32 = built.chunks.iter().map(|c| c.count).sum();
    assert_eq!(in_chunks, 13);
}

#[test]
fn a_level_that_would_pass_the_chunk_limit_is_split_and_a_chunk_is_never_over_it() {
    // 15 tiles of 40 bytes at level 2, 100 bytes a chunk: two tiles to a chunk
    let built = build(&GRID, tiles(&GRID, 40, |_, _, _| false), 0, 100, MAX).unwrap();
    let level2: Vec<_> = built.chunks.iter().filter(|c| c.level == 2).collect();
    assert_eq!(level2.len(), 8);
    assert!(level2.iter().all(|c| c.bytes.len() <= 100));
    assert_eq!(level2.iter().map(|c| c.chunk).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>());
    assert_eq!(built.index.levels()[2].chunks, 8);
    assert_eq!(built.index.levels()[0].chunks, 1);
    // each chunk is exactly its tiles, so the index can be checked against the blocks
    let index = RasterIndex::parse(&built.index.encode(), MAX).unwrap();
    let length =
        |l, c| built.chunks.iter().find(|b| b.level == l && b.chunk == c).map(|b| b.bytes.len() as u32);
    index.check_chunks(length).unwrap();
}

#[test]
fn a_tile_longer_than_a_chunk_is_refused_when_the_blocks_are_made() {
    let r = build(&GRID, tiles(&GRID, 40, |_, _, _| false), 0, 39, MAX);
    assert!(matches!(r, Err(IndexError::TooLarge { .. })), "{r:?}");
}

#[test]
fn the_coarsest_levels_can_be_inline_and_their_entries_point_into_the_index_block() {
    let built = build(&GRID, tiles(&GRID, 40, |_, _, _| false), 2, 1 << 20, MAX).unwrap();
    let bytes = built.index.encode();
    let index = RasterIndex::parse(&bytes, MAX).unwrap();
    assert_eq!(built.chunks.iter().map(|c| c.level).collect::<Vec<_>>(), vec![2]);
    assert_eq!((index.levels()[0].chunks, index.levels()[1].chunks, index.levels()[2].chunks), (0, 0, 1));
    let inline_start = HEADER_LEN + LEVEL_LEN * 3 + TILE_LEN * 20;
    assert_eq!(bytes.len(), inline_start + 5 * 40);
    for (l, c, r) in [(0, 0, 0), (1, 0, 0), (1, 1, 1)] {
        let t = index.tile(l, c, r).unwrap();
        assert!(t.is_inline() && t.offset as usize >= inline_start);
        assert_eq!(index.inline_bytes(t).unwrap(), &bytes_of(l, c, r, 40)[..]);
        assert_eq!(&bytes[t.offset as usize..(t.offset + t.length) as usize], &bytes_of(l, c, r, 40)[..]);
    }
    assert!(index.inline_bytes(index.tile(2, 0, 0).unwrap()).is_none(), "a chunked tile is not inline");
}

#[test]
fn a_level_holds_its_tiles_in_strictly_ascending_hilbert_order() {
    let index = RasterIndex::parse(&small(), MAX).unwrap();
    for (l, level) in index.levels().iter().enumerate() {
        let side = u64::from(level.cols.max(level.rows)).next_power_of_two();
        let keys: Vec<u64> =
            index.level_tiles(l as u32).unwrap().iter().map(|t| hilbert_key(side, t.col, t.row)).collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "level {l}: {keys:?}");
    }
}

#[test]
fn a_tile_outside_the_grid_or_twice_is_refused_when_the_blocks_are_made() {
    let mut given = tiles(&GRID, 40, |_, _, _| false);
    given.push(TileIn { level: 1, col: 2, row: 0, bytes: vec![1] });
    let e = build(&GRID, given, 0, 1 << 20, MAX).expect_err("outside the grid");
    assert!(matches!(e, IndexError::Tile { level: 1, col: 2, row: 0 }), "{e:?}");
    let mut twice = tiles(&GRID, 40, |_, _, _| false);
    twice.push(TileIn { level: 2, col: 1, row: 1, bytes: vec![1] });
    let e = build(&GRID, twice, 0, 1 << 20, MAX).expect_err("twice");
    assert!(matches!(e, IndexError::Tile { level: 2, col: 1, row: 1 }), "{e:?}");
}

// RASTER-02, by name: every refusal below is the block lying about itself

#[test]
fn raster_02_a_block_shorter_than_its_header_is_refused() {
    for n in 0..HEADER_LEN {
        assert!(matches!(refused(&small()[..n]), IndexError::Length { .. }));
    }
}

#[test]
fn raster_02_a_length_that_is_not_what_the_header_adds_up_to_is_refused() {
    let good = small();
    let mut longer = good.clone();
    longer.push(0);
    assert!(matches!(refused(&longer), IndexError::Length { .. }));
    assert!(matches!(refused(&good[..good.len() - 1]), IndexError::Length { .. }));
    let mut lies = good.clone();
    put(&mut lies, 20, 41); // inline_len
    assert!(matches!(refused(&lies), IndexError::Length { .. }));
}

#[test]
fn raster_02_a_count_in_the_header_is_checked_against_the_length_before_anything_is_sized_by_it() {
    // a 16 byte block that claims four billion tiles is refused without a table being allocated
    let b = header(1, u32::MAX);
    let e = RasterIndex::parse(&b, u64::MAX).expect_err("refused");
    assert!(matches!(e, IndexError::Length { .. }), "{e:?}");
    assert_eq!(e.rule(), "RASTER-02");
}

#[test]
fn raster_02_a_header_that_is_not_version_1_of_this_layout_is_refused() {
    let good = small();
    assert_eq!(&good[..4], b"PVX1");
    for (name, at, value) in [
        ("the magic", 0usize, b'Q'),
        ("the version", 4, 2),
        ("a reserved byte", 24, 1),
        ("the last reserved byte", 31, 1),
    ] {
        let mut b = good.clone();
        b[at] = value;
        assert_eq!(refused(&b), IndexError::Header, "{name}");
    }
}

#[test]
fn raster_02_the_chunk_total_of_the_header_is_the_sum_over_the_levels() {
    let mut b = small();
    put(&mut b, 16, 99);
    assert_eq!(refused(&b), IndexError::Count);
}

#[test]
fn raster_02_no_level_or_too_many_levels_are_refused() {
    for n in [0u16, 65, u16::MAX] {
        let mut b = small();
        put16(&mut b, 6, n);
        assert!(matches!(refused(&b), IndexError::Levels { found } if found == u32::from(n)));
    }
}

#[test]
fn raster_02_a_level_that_does_not_add_up_is_refused() {
    let at = |l: usize, field: usize| HEADER_LEN + LEVEL_LEN * l + 4 * field;
    for (name, edit) in [
        ("zero columns", (at(1, 0), 0u32)),
        ("zero rows", (at(1, 1), 0)),
        ("65,536 columns, which a u16 cannot name", (at(1, 0), 65_536)),
        ("more chunks than a subtype names", (at(1, 2), 1025)),
        ("a first ordinal that is not the number of entries before it", (at(1, 3), 2)),
    ] {
        let mut b = small();
        put(&mut b, edit.0, edit.1);
        let e = refused(&b);
        assert!(matches!(e, IndexError::Level { level: 1 }), "{name}: {e:?}");
    }
    // a grid that holds a number of tiles other than the header says
    let mut b = small();
    put(&mut b, HEADER_LEN + LEVEL_LEN * 2, 6); // 6 by 3 where 5 by 3 was
    assert!(matches!(refused(&b), IndexError::Count | IndexError::Level { .. }));
}

#[test]
fn raster_02_tiles_out_of_the_grid_or_out_of_order_are_refused() {
    let good = small();
    let table = HEADER_LEN + LEVEL_LEN * 3;
    let entry = |ordinal: usize| table + TILE_LEN * ordinal;
    // swap the places of the first two tiles of level 1, leaving what they hold where it is
    let mut swapped = good.clone();
    let (a, b) = (entry(1) + 12, entry(2) + 12);
    for i in 0..4 {
        swapped.swap(a + i, b + i);
    }
    let e = refused(&swapped);
    assert!(matches!(e, IndexError::Order { level: 1, .. }), "{e:?}");
    // a column outside the level
    let mut outside = good.clone();
    put(&mut outside, entry(1) + 12, 2);
    assert!(matches!(refused(&outside), IndexError::Order { level: 1, ordinal: 1 }));
    // the same tile twice
    let mut twice = good.clone();
    let (src, dst) = (entry(3), entry(4));
    let copy = twice[src..src + TILE_LEN].to_vec();
    twice[dst..dst + TILE_LEN].copy_from_slice(&copy);
    assert!(matches!(refused(&twice), IndexError::Order { .. }));
}

#[test]
fn raster_02_a_tile_that_breaks_the_packing_rules_is_refused() {
    let built = build(&GRID, tiles(&GRID, 40, |l, c, r| l == 2 && c == 4 && r == 2), 1, 100, MAX).unwrap();
    let good = built.index.encode();
    let table = HEADER_LEN + LEVEL_LEN * 3;
    let entry = |ordinal: u32| table + TILE_LEN * ordinal as usize;
    let level2 = built.index.levels()[2].first;
    // a chunked tile of level 2 that is the second of its chunk
    let second_in_chunk =
        (level2..level2 + 15).find(|&o| built.index.tiles()[o as usize].offset == 40).unwrap();
    let (offset, length, chunk) = (4, 8, 0);
    for (name, field, value) in [
        ("an offset inside the previous tile", offset, 20u32),
        ("an offset with a gap before it", offset, 41),
        ("an offset past the end", offset, u32::MAX),
        ("a chunk the level does not have", chunk, 99),
        ("a chunk that skips one", chunk, built.index.tiles()[second_in_chunk as usize].chunk + 2),
    ] {
        let mut b = good.clone();
        put(&mut b, entry(second_in_chunk) + field, value);
        let e = refused(&b);
        assert!(matches!(e, IndexError::Packing { .. }), "{name}: {e:?}");
    }
    // the tile before it grows, so the next one no longer starts where it ends
    let mut longer = good.clone();
    put(&mut longer, entry(second_in_chunk - 1) + length, 41);
    assert!(matches!(refused(&longer), IndexError::Packing { .. }));
    // a tile with no length must carry nothing else
    let unset = level2 + (0..15).find(|&k| built.index.tiles()[(level2 + k) as usize].length == 0).unwrap();
    for (name, field, value) in [("a checksum", 16usize, 1u32), ("an offset", 4, 1), ("a chunk", 0, 1)] {
        let mut b = good.clone();
        put(&mut b, entry(unset) + field, value);
        let e = refused(&b);
        assert!(matches!(e, IndexError::Packing { .. }), "{name} on an unstored tile: {e:?}");
    }
    // an inline tile in a level that names chunks
    let mut inline_in_chunked = good.clone();
    put(&mut inline_in_chunked, entry(second_in_chunk), INLINE);
    assert!(matches!(refused(&inline_in_chunked), IndexError::Packing { .. }));
}

#[test]
fn raster_02_the_stored_count_and_the_inline_part_must_be_what_the_tiles_say() {
    let good = small();
    let mut count = good.clone();
    put(&mut count, 12, 19);
    assert_eq!(refused(&count), IndexError::Count);
    // inline bytes no entry points at
    let mut spare = good.clone();
    spare.extend_from_slice(&[7; 8]);
    put(&mut spare, 20, 40 + 8);
    assert_eq!(refused(&spare), IndexError::Count);
}

#[test]
fn raster_02_the_levels_must_be_the_pyramid_the_placement_fixes() {
    // 1280 by 768 pixels in tiles of 256 is 5 by 3 tiles at full size, and each level above is halved with a
    // ceiling until the first level of one tile: 640 by 384, 320 by 192, 160 by 96
    let grid: Vec<(u32, u32)> = (0..4).map(|l| grid_of(1280, 768, 256, 4, l)).collect();
    assert_eq!(grid, [(1, 1), (2, 1), (3, 2), (5, 3)]);
    let built = build(&grid, Vec::new(), 0, 1 << 20, MAX).unwrap();
    let index = RasterIndex::parse(&built.index.encode(), MAX).unwrap();
    index.check_grid(1280, 768, 256).unwrap();
    // any size that rounds to the same grid at every level is the same pyramid
    index.check_grid(1257, 700, 256).unwrap();
    for (w, h, edge) in
        [(1280, 1025, 256), (1280, 768, 512), (2000, 768, 256), (1280, 700, 128), (1024, 768, 256)]
    {
        let e = index.check_grid(w, h, edge).expect_err(&format!("{w}x{h}/{edge}"));
        assert!(matches!(e, IndexError::Grid { .. }), "{w}x{h}/{edge}: {e:?}");
        assert_eq!(e.rule(), "RASTER-02");
    }
}

#[test]
fn raster_02_a_chunk_block_must_be_exactly_its_tiles() {
    let built = build(&GRID, tiles(&GRID, 40, |_, _, _| false), 1, 100, MAX).unwrap();
    let index = RasterIndex::parse(&built.index.encode(), MAX).unwrap();
    let real = |l: u32, c: u32| {
        built.chunks.iter().find(|b| b.level == l && b.chunk == c).map(|b| b.bytes.len() as u32)
    };
    index.check_chunks(real).unwrap();
    let missing = |l, c| if (l, c) == (2, 3) { None } else { real(l, c) };
    let short = |l, c| real(l, c).map(|n| if (l, c) == (2, 0) { n - 1 } else { n });
    let long = |l, c| real(l, c).map(|n| if (l, c) == (1, 0) { n + 1 } else { n });
    assert!(matches!(index.check_chunks(missing), Err(IndexError::Chunk { level: 2, chunk: 3 })));
    assert!(matches!(index.check_chunks(short), Err(IndexError::Chunk { level: 2, chunk: 0 })));
    assert!(matches!(index.check_chunks(long), Err(IndexError::Chunk { level: 1, chunk: 0 })));
}

#[test]
fn limit_14_131073_tiles_are_refused_under_the_browser_ceiling_and_accepted_under_the_default() {
    // two levels of 65,535 by 2 and 1 by 3, nothing stored: the table is the size
    let grid = [(65_535u32, 2u32), (1, 3)];
    let browser = 1u64 << 17;
    let default = 1u64 << 20;
    let e = build(&grid, Vec::new(), 0, 1 << 20, browser).expect_err("refused");
    assert_eq!(e.rule(), "LIMIT-14");
    assert!(matches!(e, IndexError::TooManyTiles { declared: 131_073, ceiling } if ceiling == browser));
    let built = build(&grid, Vec::new(), 0, 1 << 20, default).unwrap();
    let bytes = built.index.encode();
    assert_eq!(bytes.len(), HEADER_LEN + 2 * LEVEL_LEN + TILE_LEN * 131_073);
    let e = RasterIndex::parse(&bytes, browser).expect_err("refused");
    assert_eq!(e.rule(), "LIMIT-14");
    assert_eq!(RasterIndex::parse(&bytes, default).unwrap().tiles().len(), 131_073);
    assert_eq!(RasterIndex::parse(&bytes, 131_073).unwrap().stored(), 0);
}

// the inline levels are the coarsest ones, and a chunk's numbers are one pass over the table

#[test]
fn raster_02_an_inline_level_after_a_level_in_chunks_is_refused() {
    // two levels of one tile each: level 0 in chunk 0, level 1 inline, which no writer makes
    let tile = |chunk: u32, offset: u32| {
        let mut e = Vec::new();
        for v in [chunk, offset, 3] {
            e.extend_from_slice(&v.to_le_bytes());
        }
        e.extend_from_slice(&[0, 0, 0, 0]); // col, row
        e.extend_from_slice(&pvec_container::integrity::block_checksum(&[1, 2, 3]).to_le_bytes());
        e
    };
    let mut b = header(2, 2);
    put(&mut b, 12, 2); // tile_stored
    put(&mut b, 16, 1); // chunk_total
    put(&mut b, 20, 3); // inline_len
    for (cols, rows, chunks, first) in [(1u32, 1u32, 1u32, 0u32), (1, 1, 0, 1)] {
        for v in [cols, rows, chunks, first] {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    let inline_at = (b.len() + 2 * TILE_LEN) as u32;
    b.extend(tile(0, 0));
    b.extend(tile(INLINE, inline_at));
    b.extend_from_slice(&[1, 2, 3]);
    assert!(matches!(refused(&b), IndexError::Packing { ordinal: 1 }), "{:?}", refused(&b));
    // the mirror image, inline first and then chunks, is what the writer makes and parses
    let mut ok = header(2, 2);
    put(&mut ok, 12, 2);
    put(&mut ok, 16, 1);
    put(&mut ok, 20, 3);
    for (cols, rows, chunks, first) in [(1u32, 1u32, 0u32, 0u32), (1, 1, 1, 1)] {
        for v in [cols, rows, chunks, first] {
            ok.extend_from_slice(&v.to_le_bytes());
        }
    }
    ok.extend(tile(INLINE, inline_at));
    ok.extend(tile(0, 0));
    ok.extend_from_slice(&[1, 2, 3]);
    RasterIndex::parse(&ok, MAX).expect("inline first, then chunks");
}

#[test]
fn the_chunk_stats_of_a_level_are_the_tiles_and_the_end_of_each_chunk() {
    let built = build(&GRID, tiles(&GRID, 40, |_, _, _| false), 1, 100, MAX).unwrap();
    let index = RasterIndex::parse(&built.index.encode(), MAX).unwrap();
    assert!(index.chunk_stats(0).is_empty(), "an inline level has no chunk");
    assert!(index.chunk_stats(9).is_empty(), "nor does a level that is not there");
    let stats = index.chunk_stats(2);
    assert_eq!(stats.len(), 8);
    // 15 tiles of 40 bytes, two to a chunk: seven chunks of 80 and a last one of 40
    assert_eq!(stats.iter().map(|s| s.0).sum::<u32>(), 15);
    assert!(stats[..7].iter().all(|s| *s == (2, 80)) && stats[7] == (1, 40), "{stats:?}");
}
