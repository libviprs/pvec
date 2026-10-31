//! Issue #218 (R1.4): `docs/RASTER.md` section 2 is the layout of the index block, and the example in section 7 is
//! a real index block the code writes today. The walkthrough reads the bytes out of the document, not out of the
//! writer, and decodes the header, the level table and the tile table by the offsets the document gives.
#![allow(missing_docs)]

use pvec_container::blocks::raster::{INLINE, TileIn, build};
use std::fs;
use std::path::PathBuf;

const BEGIN: &str = "<!-- raster-example:begin -->";
const END: &str = "<!-- raster-example:end -->";

fn doc() -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/RASTER.md"))
        .expect("RASTER.md")
}

/// The example: a 300 by 300 image in tiles of 256 is two levels, 1 by 1 and 2 by 2. Level 0 is inline, level 1
/// is in one chunk, and the tile at column 1, row 0 of level 1 is not stored. Tile bytes are three bytes each.
fn example() -> Vec<u8> {
    let mut tiles = vec![TileIn { level: 0, col: 0, row: 0, bytes: vec![0xA0, 0xA1, 0xA2] }];
    for (i, (col, row)) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
        let bytes = if (col, row) == (1, 0) { Vec::new() } else { vec![0xB0 + i as u8; 3] };
        tiles.push(TileIn { level: 1, col, row, bytes });
    }
    build(&[(1, 1), (2, 2)], tiles, 1, 1 << 20, 1 << 20).unwrap().index.encode()
}

fn dump(bytes: &[u8]) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{:08x}  {}\n",
                i * 16,
                c.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
            )
        })
        .collect()
}

fn bytes_in_doc() -> Vec<u8> {
    let d = doc();
    let (from, to) = (d.find(BEGIN).expect("no example in RASTER.md"), d.find(END).expect("no end marker"));
    let mut out = Vec::new();
    for line in d[from + BEGIN.len()..to].lines() {
        let Some((offset, rest)) = line.split_once("  ") else { continue };
        if offset.len() != 8 || u64::from_str_radix(offset, 16).is_err() {
            continue;
        }
        assert_eq!(usize::from_str_radix(offset, 16).unwrap(), out.len(), "dump line out of order: {line}");
        out.extend(rest.split_whitespace().map(|h| u8::from_str_radix(h, 16).expect("a hex byte")));
    }
    out
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

#[test]
fn print_the_dump_for_the_document() {
    if std::env::var_os("PVEC_PRINT_RASTER_EXAMPLE").is_some() {
        println!("RASTER-EXAMPLE-BEGIN\n{}RASTER-EXAMPLE-END", dump(&example()));
    }
}

#[test]
fn the_dump_in_the_document_is_the_index_block_the_writer_produces() {
    assert_eq!(bytes_in_doc(), example());
}

#[test]
fn the_layout_decodes_the_dump_by_the_offsets_of_section_2_alone() {
    let b = bytes_in_doc();
    // header, 32 bytes
    assert_eq!(&b[..4], b"PVX1");
    assert_eq!(u16_at(&b, 4), 1, "version");
    let levels = usize::from(u16_at(&b, 6));
    let (total, stored, chunk_total, inline_len) =
        (u32_at(&b, 8), u32_at(&b, 12), u32_at(&b, 16), u32_at(&b, 20));
    assert!(b[24..32].iter().all(|x| *x == 0), "8 reserved bytes");
    assert_eq!((levels, total, stored, chunk_total, inline_len), (2, 5, 4, 1, 3));
    // level table, 16 bytes a level, straight after the header
    let level = |l: usize| {
        let at = 32 + 16 * l;
        (u32_at(&b, at), u32_at(&b, at + 4), u32_at(&b, at + 8), u32_at(&b, at + 12))
    };
    assert_eq!(level(0), (1, 1, 0, 0), "cols rows chunks first: level 0 is inline");
    assert_eq!(level(1), (2, 2, 1, 1), "level 1 is one chunk and starts at tile 1");
    // tile table, 24 bytes a tile, straight after the level table
    let table = 32 + 16 * levels;
    let tile = |n: usize| {
        let at = table + 24 * n;
        (
            u32_at(&b, at),
            u32_at(&b, at + 4),
            u32_at(&b, at + 8),
            u16_at(&b, at + 12),
            u16_at(&b, at + 14),
            u64_at(&b, at + 16),
        )
    };
    // chunk, offset, length, col, row, xxh3
    let inline_at = table + 24 * total as usize;
    let t0 = tile(0);
    assert_eq!(
        (t0.0, t0.1 as usize, t0.2, t0.3, t0.4),
        (INLINE, inline_at, 3, 0, 0),
        "inline: offset into the index block"
    );
    assert_eq!(&b[inline_at..inline_at + 3], &[0xA0, 0xA1, 0xA2]);
    assert_eq!(t0.5, pvec_container::integrity::block_checksum(&[0xA0, 0xA1, 0xA2]));
    // level 1 in Hilbert order with the first step along x: (0,0) (1,0) (1,1) (0,1)
    let order: Vec<(u16, u16)> = (1..5).map(|n| (tile(n).3, tile(n).4)).collect();
    assert_eq!(order, [(0, 0), (1, 0), (1, 1), (0, 1)]);
    // the tile at (1, 0) is not stored: length 0 and everything else 0
    let sparse = tile(2);
    assert_eq!((sparse.0, sparse.1, sparse.2, sparse.5), (0, 0, 0, 0));
    // the others are in chunk 0, packed one after the other
    assert_eq!([tile(1).1, tile(3).1, tile(4).1], [0, 3, 6]);
    assert!([tile(1), tile(3), tile(4)].iter().all(|t| t.0 == 0 && t.2 == 3));
    assert_eq!(b.len(), inline_at + inline_len as usize, "the inline part is the end of the block");
}
