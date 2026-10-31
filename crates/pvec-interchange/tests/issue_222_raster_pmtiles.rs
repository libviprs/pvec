//! The PMTiles raster passthrough, archive layer (issue R2.3, pvec#222).
//!
//! Red first: these describe `pvec_interchange::pmtiles::{write_raster_archive, read_raster_archive}` before
//! they exist. The archive is checked by `tests/pmtiles_ref`, a reader written from the PMTiles v3
//! specification that shares no code with the writer, and the hostile cases are built here by hand with
//! bytes the writer would never produce (a gzip member, run lengths, a leaf that points at itself).
//! The placement layer on top of this one (the `vnd.pvec` key as a placement) is in the same file once the
//! placement type of the stream exists.

mod pmtiles_ref;
mod report_json;

use pmtiles_ref::*;
use pvec_interchange::mvt::{MvtEntity, MvtOptions, Projection, TileId, export_tile};
use pvec_interchange::pmtiles::{
    PmtilesOptions, Provenance, RasterArchive, RasterArchiveError, RasterReadLimits, RasterTile,
    RasterTileType, read_raster_archive, write_pmtiles, write_raster_archive,
};
use report_json::{J, json};

// ---- fixtures -------------------------------------------------------------------------------------------

/// A made-up tile payload: distinct per address so a swap shows, never decoded by anything here.
fn payload(z: u8, x: u32, y: u32) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G'];
    v.extend_from_slice(&[z, x as u8, y as u8, (x * 31 + y * 7 + u32::from(z)) as u8]);
    v.extend((0..(5 + (x + 2 * y) as usize)).map(|i| (i as u8).wrapping_mul(z + 3)));
    v
}

fn tile(z: u8, x: u32, y: u32) -> RasterTile {
    RasterTile { z, x, y, bytes: payload(z, x, y) }
}

/// A 3000 by 1900 image at edge 1024 has three levels: 3 by 2 tiles at z=2, 2 by 1 at z=1, 1 by 1 at z=0.
/// The pyramid is sparse: z=2 has no tile at (1, 0) and z=1 has no tile at (1, 0).
fn sparse_three_levels() -> Vec<RasterTile> {
    let mut t = vec![tile(0, 0, 0), tile(1, 0, 0)];
    for (x, y) in [(0, 0), (2, 0), (0, 1), (1, 1), (2, 1)] {
        t.push(tile(2, x, y));
    }
    t
}

fn archive(tiles: Vec<RasterTile>) -> RasterArchive {
    RasterArchive { tile_type: RasterTileType::Png, tiles, pvec: None, attribution: None }
}

fn meta(a: &[u8]) -> J {
    let h = header(a);
    json(std::str::from_utf8(range(a, h.meta)).unwrap())
}

// ---- the writer -----------------------------------------------------------------------------------------

#[test]
fn issue_222_the_directory_has_one_entry_per_stored_tile_and_none_for_a_missing_one() {
    let a = write_raster_archive(&archive(sparse_three_levels())).unwrap();
    let got = all_tiles(&a);
    assert_eq!(got.len(), 7);
    let mut want: Vec<(u64, Vec<u8>)> = sparse_three_levels()
        .iter()
        .map(|t| (ref_tile_id(u32::from(t.z), u64::from(t.x), u64::from(t.y)), t.bytes.clone()))
        .collect();
    want.sort();
    assert_eq!(got, want, "every tile at its hand-computed id, bytes unchanged");
    for (z, x, y) in [(2u32, 1u64, 0u64), (1, 1, 0)] {
        assert!(find(&a, ref_tile_id(z, x, y)).is_none(), "no entry for the tile not stored at {z}/{x}/{y}");
    }
    let h = header(&a);
    assert_eq!((h.addressed, h.entries, h.contents), (7, 7, 7));
    assert_eq!((h.min_zoom, h.max_zoom), (0, 2));
}

#[test]
fn issue_222_z_x_y_are_the_ones_computed_by_hand_for_a_three_level_pyramid() {
    // PMTiles tile ids: z=0 is 0, z=1 is 1..=4 along the Hilbert curve, z=2 starts at 5.
    assert_eq!(ref_tile_id(0, 0, 0), 0);
    assert_eq!(ref_tile_id(1, 0, 0), 1);
    assert_eq!(ref_tile_id(1, 0, 1), 2);
    assert_eq!(ref_tile_id(1, 1, 1), 3);
    assert_eq!(ref_tile_id(1, 1, 0), 4);
    assert_eq!(ref_tile_id(2, 0, 0), 5);
    let a = write_raster_archive(&archive(sparse_three_levels())).unwrap();
    for t in sparse_three_levels() {
        let id = ref_tile_id(u32::from(t.z), u64::from(t.x), u64::from(t.y));
        assert_eq!(find(&a, id).unwrap(), &t.bytes[..], "{}/{}/{}", t.z, t.x, t.y);
    }
}

#[test]
fn issue_222_the_header_names_the_tile_type_and_leaves_the_tiles_uncompressed() {
    for (ty, byte) in [(RasterTileType::Png, 2u8), (RasterTileType::Jpeg, 3), (RasterTileType::Webp, 4)] {
        let mut ar = archive(sparse_three_levels());
        ar.tile_type = ty;
        let a = write_raster_archive(&ar).unwrap();
        let h = header(&a);
        assert_eq!(h.tile_type, byte);
        assert_eq!(h.tile_compression, 1, "none: the tile codec has compressed them already");
        assert_eq!(h.internal_compression, 1, "none");
        assert_eq!(h.clustered, 1);
    }
}

#[test]
fn issue_222_the_header_bounds_are_the_whole_world_square_like_the_vector_export() {
    let a = write_raster_archive(&archive(sparse_three_levels())).unwrap();
    let i = |at: usize| i32::from_le_bytes(a[at..at + 4].try_into().unwrap());
    assert_eq!([i(102), i(106), i(110), i(114)], [-1_800_000_000, -850_511_288, 1_800_000_000, 850_511_288]);
}

#[test]
fn issue_222_a_zero_length_tile_gets_no_entry() {
    let mut t = sparse_three_levels();
    t.push(RasterTile { z: 1, x: 1, y: 0, bytes: Vec::new() });
    let a = write_raster_archive(&archive(t)).unwrap();
    assert_eq!(all_tiles(&a).len(), 7);
    assert!(find(&a, ref_tile_id(1, 1, 0)).is_none());
    assert_eq!(header(&a).addressed, 7);
}

#[test]
fn issue_222_the_same_tiles_in_any_order_give_the_same_archive() {
    let mut t = sparse_three_levels();
    let first = write_raster_archive(&archive(t.clone())).unwrap();
    t.reverse();
    assert_eq!(write_raster_archive(&archive(t)).unwrap(), first);
    assert_eq!(write_raster_archive(&archive(sparse_three_levels())).unwrap(), first);
}

#[test]
fn issue_222_the_placement_key_and_the_attribution_are_in_the_metadata_and_round_trip() {
    let mut ar = archive(sparse_three_levels());
    ar.pvec =
        Some(r#"{"raster":{"width":3000,"pixel_to_source":["3ff0000000000000","0000000000000000"]}}"#.into());
    ar.attribution = Some("Survey 2026 <b>plain</b> text".into());
    let a = write_raster_archive(&ar).unwrap();
    let m = meta(&a);
    assert_eq!(m.get("attribution").str(), "Survey 2026 <b>plain</b> text");
    assert_eq!(m.get("vnd.pvec").get("raster").get("width").num(), 3000.0);
    let back = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    assert_eq!(back.attribution, ar.attribution);
    let again = json(back.pvec.as_deref().unwrap());
    assert_eq!(again, json(ar.pvec.as_deref().unwrap()), "the key reads back as the same JSON");
}

#[test]
fn issue_222_no_metadata_member_is_written_for_what_the_caller_did_not_give() {
    let a = write_raster_archive(&archive(sparse_three_levels())).unwrap();
    let m = meta(&a);
    assert!(!m.has("vnd.pvec") && !m.has("attribution"));
}

#[test]
fn issue_222_a_tile_given_twice_an_address_outside_its_zoom_and_a_bad_key_are_refused() {
    let mut t = sparse_three_levels();
    t.push(tile(2, 0, 0));
    assert_eq!(
        write_raster_archive(&archive(t)).unwrap_err(),
        RasterArchiveError::DuplicateTile { z: 2, x: 0, y: 0 }
    );
    let t = vec![tile(1, 2, 0)];
    assert_eq!(
        write_raster_archive(&archive(t)).unwrap_err(),
        RasterArchiveError::TileOutsideZoom { z: 1, x: 2, y: 0 }
    );
    let t = vec![tile(32, 0, 0)];
    assert_eq!(write_raster_archive(&archive(t)).unwrap_err(), RasterArchiveError::ZoomTooDeep(32));
    for bad in ["[1]", "not json", "{\"a\":1,\"a\":2}", "{\"a\":"] {
        let mut ar = archive(sparse_three_levels());
        ar.pvec = Some(bad.into());
        assert_eq!(write_raster_archive(&ar).unwrap_err(), RasterArchiveError::BadPlacementKey, "{bad}");
    }
}

#[test]
fn issue_222_many_tiles_move_into_leaf_directories_and_the_root_still_fits_in_16_kib() {
    let mut t = Vec::new();
    for y in 0..64u32 {
        for x in 0..64u32 {
            t.push(tile(6, x, y));
        }
    }
    let a = write_raster_archive(&archive(t.clone())).unwrap();
    let h = header(&a);
    assert!(h.leaves.1 > 0, "this many entries do not fit the root");
    assert!(h.root.0 + h.root.1 <= 16_384);
    assert_eq!(all_tiles(&a).len(), 4096);
    let back = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
    assert_eq!(back.tiles.len(), 4096);
}

// ---- the reader -----------------------------------------------------------------------------------------

fn sorted(mut t: Vec<RasterTile>) -> Vec<RasterTile> {
    t.sort_by_key(|t| (t.z, t.y, t.x));
    t
}

#[test]
fn issue_222_what_the_writer_wrote_reads_back_with_every_tile_byte_for_byte() {
    for ty in [RasterTileType::Png, RasterTileType::Jpeg, RasterTileType::Webp] {
        let mut ar = archive(sparse_three_levels());
        ar.tile_type = ty;
        let a = write_raster_archive(&ar).unwrap();
        let back = read_raster_archive(&a, &RasterReadLimits::default()).unwrap();
        assert_eq!(back.tile_type, ty);
        assert_eq!(sorted(back.tiles), sorted(sparse_three_levels()));
        assert_eq!((back.pvec, back.attribution), (None, None));
    }
}

// An independent archive builder: gzip directories, a leaf directory, a run length and shared offsets,
// built from the PMTiles specification with stored (uncompressed) deflate blocks.

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    let mut chunks = data.chunks(65_535).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(c) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        out.extend_from_slice(&(c.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// (id, offset, length, run) entries as a directory.
fn dir_bytes(entries: &[(u64, u64, u64, u64)]) -> Vec<u8> {
    let mut o = Vec::new();
    put_varint(&mut o, entries.len() as u64);
    let mut last = 0;
    for e in entries {
        put_varint(&mut o, e.0 - last);
        last = e.0;
    }
    for e in entries {
        put_varint(&mut o, e.3);
    }
    for e in entries {
        put_varint(&mut o, e.2);
    }
    for e in entries {
        put_varint(&mut o, e.1 + 1);
    }
    o
}

struct Parts {
    root: Vec<u8>,
    meta: Vec<u8>,
    leaves: Vec<u8>,
    data: Vec<u8>,
    internal: u8,
    tile_comp: u8,
    tile_type: u8,
}

fn build(p: &Parts) -> Vec<u8> {
    let root_off = 127u64;
    let meta_off = root_off + p.root.len() as u64;
    let leaf_off = meta_off + p.meta.len() as u64;
    let data_off = leaf_off + p.leaves.len() as u64;
    let mut out = b"PMTiles".to_vec();
    out.push(3);
    for v in [
        root_off,
        p.root.len() as u64,
        meta_off,
        p.meta.len() as u64,
        leaf_off,
        p.leaves.len() as u64,
        data_off,
        p.data.len() as u64,
        1,
        1,
        1,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[1, p.internal, p.tile_comp, p.tile_type, 0, 2]);
    for v in [-1_800_000_000i32, -850_511_288, 1_800_000_000, 850_511_288] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(0);
    out.extend_from_slice(&[0; 8]);
    assert_eq!(out.len(), 127);
    out.extend_from_slice(&p.root);
    out.extend_from_slice(&p.meta);
    out.extend_from_slice(&p.leaves);
    out.extend_from_slice(&p.data);
    out
}

fn simple_parts() -> Parts {
    let t = payload(0, 0, 0);
    Parts {
        root: dir_bytes(&[(0, 0, t.len() as u64, 1)]),
        meta: b"{}".to_vec(),
        leaves: Vec::new(),
        data: t,
        internal: 1,
        tile_comp: 1,
        tile_type: 2,
    }
}

fn read(a: &[u8]) -> Result<RasterArchive, RasterArchiveError> {
    read_raster_archive(a, &RasterReadLimits::default())
}

#[test]
fn issue_222_an_archive_with_gzip_directories_a_leaf_and_run_lengths_reads() {
    // z=1 ids 1..=4 are one tile's bytes repeated four times by a run length of 4; z=0 is its own tile.
    let t0 = payload(0, 0, 0);
    let t1 = payload(1, 0, 0);
    let leaf = dir_bytes(&[(1, t0.len() as u64, t1.len() as u64, 4)]);
    let leaf_gz = gzip_stored(&leaf);
    let root = dir_bytes(&[(0, 0, t0.len() as u64, 1), (1, 0, leaf_gz.len() as u64, 0)]);
    let mut data = t0.clone();
    data.extend_from_slice(&t1);
    let p = Parts {
        root: gzip_stored(&root),
        meta: gzip_stored(br#"{"attribution":"a"}"#),
        leaves: leaf_gz,
        data,
        internal: 2,
        tile_comp: 1,
        tile_type: 3,
    };
    let got = read(&build(&p)).unwrap();
    assert_eq!(got.tile_type, RasterTileType::Jpeg);
    assert_eq!(got.attribution.as_deref(), Some("a"));
    assert_eq!(got.tiles.len(), 5);
    let z1: Vec<&RasterTile> = got.tiles.iter().filter(|t| t.z == 1).collect();
    assert_eq!(z1.len(), 4);
    assert!(z1.iter().all(|t| t.bytes == t1), "a run length expands to one tile per address, same bytes");
    assert_eq!(got.tiles.iter().find(|t| t.z == 0).unwrap().bytes, t0);
}

#[test]
fn issue_222_a_vector_archive_is_not_a_raster_and_raw_and_unknown_types_are_refused() {
    let p = |x: f64| pvec_geometry::Point2::new(x, x);
    let ents = vec![
        MvtEntity::new(
            pvec_entity::EntityId::new(1).unwrap(),
            "l",
            pvec_geometry::Curve::Polyline(pvec_geometry::Polyline::new(vec![p(1.0), p(90.0)]).unwrap()),
        )
        .unwrap(),
    ];
    let pr = Projection::new(pvec_geometry::Transform2::IDENTITY, 256.0, 256.0).unwrap();
    let tiles = vec![export_tile(&ents, &pr, TileId::new(0, 0, 0).unwrap(), &MvtOptions::default()).unwrap()];
    let prov = Provenance {
        source_digest: Some([1; 32]),
        tessellation_tolerance_px: Some(0.25),
        tested_zooms: Some((0, 0)),
        created: None,
    };
    let vector = write_pmtiles(&tiles, &prov, &PmtilesOptions::default()).unwrap();
    assert_eq!(read(&vector).unwrap_err(), RasterArchiveError::NotARaster);

    for (ty, want) in [
        (0u8, RasterArchiveError::RawTileType),
        (1, RasterArchiveError::NotARaster),
        (5, RasterArchiveError::UnsupportedTileType(5)),
        (9, RasterArchiveError::UnsupportedTileType(9)),
    ] {
        let mut s = simple_parts();
        s.tile_type = ty;
        assert_eq!(read(&build(&s)).unwrap_err(), want, "tile type {ty}");
    }
}

#[test]
fn issue_222_a_compressed_tile_or_an_unknown_directory_compression_is_refused() {
    for tc in [0u8, 2, 3, 4, 9] {
        let mut s = simple_parts();
        s.tile_comp = tc;
        assert_eq!(
            read(&build(&s)).unwrap_err(),
            RasterArchiveError::TileCompression(tc),
            "tile compression {tc}"
        );
    }
    for ic in [0u8, 3, 4, 9] {
        let mut s = simple_parts();
        s.internal = ic;
        assert_eq!(
            read(&build(&s)).unwrap_err(),
            RasterArchiveError::InternalCompression(ic),
            "internal {ic}"
        );
    }
}

#[test]
fn issue_222_a_damaged_header_or_regions_outside_the_file_are_refused_not_trusted() {
    let good = build(&simple_parts());
    assert!(read(&good).is_ok());
    assert_eq!(read(&good[..100]).unwrap_err(), RasterArchiveError::NotAnArchive);
    let mut bad = good.clone();
    bad[0] = b'X';
    assert_eq!(read(&bad).unwrap_err(), RasterArchiveError::NotAnArchive);
    let mut bad = good.clone();
    bad[7] = 2;
    assert_eq!(read(&bad).unwrap_err(), RasterArchiveError::NotAnArchive);
    for field in [8usize, 24, 40, 56] {
        // the offset, then the length of each region, pushed past the end or overflowing
        for (at, v) in [(field, u64::MAX), (field + 8, u64::MAX), (field, 1 << 40), (field + 8, 1 << 40)] {
            let mut bad = good.clone();
            bad[at..at + 8].copy_from_slice(&v.to_le_bytes());
            assert!(matches!(read(&bad), Err(RasterArchiveError::RegionOutsideFile(_))), "field at {at}");
        }
    }
    assert!(read(&good[..good.len() - 1]).is_err(), "a truncated data region");
}

#[test]
fn issue_222_a_tile_that_points_outside_the_data_region_is_refused() {
    let mut s = simple_parts();
    s.root = dir_bytes(&[(0, 0, s.data.len() as u64 + 1, 1)]);
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::TileOutsideData { .. })));
    let mut s = simple_parts();
    s.root = dir_bytes(&[(0, u64::MAX - 3, 8, 1)]);
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::TileOutsideData { .. })));
}

#[test]
fn issue_222_a_directory_that_claims_more_entries_than_it_has_bytes_is_refused_before_allocating() {
    let mut s = simple_parts();
    let mut d = Vec::new();
    put_varint(&mut d, 1 << 40);
    s.root = d;
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))));
    let mut s = simple_parts();
    s.root = vec![0x80; 20];
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))));
    let mut s = simple_parts();
    s.root.push(0);
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))), "trailing bytes");
}

#[test]
fn issue_222_a_leaf_that_points_at_itself_is_refused() {
    let leaf = dir_bytes(&[(0, 0, 0, 0)]);
    let mut s = simple_parts();
    s.leaves = leaf.clone();
    s.root = dir_bytes(&[(0, 0, leaf.len() as u64, 0)]);
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))));
}

#[test]
fn issue_222_a_run_length_cannot_multiply_the_bytes_past_the_limits() {
    let t = payload(0, 0, 0);
    let mut s = simple_parts();
    s.root = dir_bytes(&[(1, 0, t.len() as u64, 4)]);
    let lim = RasterReadLimits { max_tiles: 3, ..RasterReadLimits::default() };
    assert_eq!(
        read_raster_archive(&build(&s), &lim).unwrap_err(),
        RasterArchiveError::TooManyTiles { limit: 3 }
    );
    let lim = RasterReadLimits { max_total_bytes: 3 * t.len() as u64, ..RasterReadLimits::default() };
    assert!(matches!(read_raster_archive(&build(&s), &lim), Err(RasterArchiveError::TooManyBytes { .. })));
    // a huge run is refused by arithmetic, not by trying to allocate
    let mut s = simple_parts();
    s.root = dir_bytes(&[(1, 0, t.len() as u64, u64::from(u32::MAX) * 4096)]);
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::TooManyTiles { .. })));
}

#[test]
fn issue_222_one_tile_over_the_per_tile_cap_is_refused() {
    let t = payload(0, 0, 0);
    let lim = RasterReadLimits { max_tile_bytes: t.len() as u64 - 1, ..RasterReadLimits::default() };
    assert!(matches!(
        read_raster_archive(&build(&simple_parts()), &lim),
        Err(RasterArchiveError::TileTooLarge { .. })
    ));
}

#[test]
fn issue_222_a_tile_id_past_zoom_31_or_not_in_ascending_order_is_refused() {
    let t = payload(0, 0, 0);
    let mut s = simple_parts();
    s.root = dir_bytes(&[(u64::MAX / 2, 0, t.len() as u64, 1)]);
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))));
    let mut s = simple_parts();
    s.root = {
        let mut d = Vec::new();
        put_varint(&mut d, 2);
        for v in [5u64, 0] {
            put_varint(&mut d, v); // ids 5 then 5 again (delta zero)
        }
        for _ in 0..2 {
            put_varint(&mut d, 1);
        }
        for _ in 0..2 {
            put_varint(&mut d, t.len() as u64);
        }
        for _ in 0..2 {
            put_varint(&mut d, 1);
        }
        d
    };
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))), "a repeated id");
}

#[test]
fn issue_222_the_metadata_is_bounded_and_must_be_an_object_with_an_object_key() {
    let lim = RasterReadLimits { max_metadata_bytes: 1, ..RasterReadLimits::default() };
    assert!(matches!(
        read_raster_archive(&build(&simple_parts()), &lim),
        Err(RasterArchiveError::BadMetadata(_))
    ));
    for bad in
        [&b"[]"[..], b"{\"vnd.pvec\":1}", b"{\"attribution\":3}", b"{\"a\":1,\"a\":2}", b"{", b"\xff\xfe"]
    {
        let mut s = simple_parts();
        s.meta = bad.to_vec();
        assert!(
            matches!(read(&build(&s)), Err(RasterArchiveError::BadMetadata(_))),
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let mut s = simple_parts();
    s.meta = format!("{{\"x\":{deep}}}").into_bytes();
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadMetadata(_))), "depth is bounded");
}

#[test]
fn issue_222_a_gzip_bomb_in_a_directory_is_cut_off_at_the_limit() {
    // a deflate stream of stored blocks whose declared size is far above the cap
    let mut s = simple_parts();
    s.internal = 2;
    let big = vec![0u8; 3 << 20];
    s.root = gzip_stored(&big);
    s.meta = gzip_stored(b"{}");
    let lim = RasterReadLimits { max_metadata_bytes: 1 << 20, ..RasterReadLimits::default() };
    assert!(read_raster_archive(&build(&s), &lim).is_err());
    let mut s = simple_parts();
    s.internal = 2;
    s.root = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255, 1, 2, 3];
    assert!(matches!(read(&build(&s)), Err(RasterArchiveError::BadDirectory(_))));
}

#[test]
fn issue_222_nothing_in_a_random_buffer_panics() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let good = build(&simple_parts());
    for round in 0..3000 {
        let mut b = good.clone();
        for _ in 0..(1 + round % 5) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let at = (x % b.len() as u64) as usize;
            b[at] = (x >> 32) as u8;
        }
        let _ = read(&b);
    }
}
