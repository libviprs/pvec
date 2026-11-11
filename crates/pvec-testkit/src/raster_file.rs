//! A whole artifact with rasters in it (issue pvec#220): one line of geometry, the placement stream of
//! capability 21, and for each raster the index block and the tile blocks that `blocks::raster::build` makes.
//! It is what the raster plan's tests, the WASM export tests and the measurement draw their files from, so
//! the three read the same bytes. Test-only: nothing in a shipped crate calls it.

use pvec_components::{Placement, RASTER_CAPABILITY, Stream, encode_rasters};
use pvec_container::blocks::raster::{CAPABILITY, KIND_INDEX, KIND_TILES, TileIn, build, grid_of, subtype};
use pvec_container::directory::{FLAG_HAS_BOUNDS, parse_directory};
use pvec_container::integrity::Integrity;
use pvec_container::writer::{ContainerWriter, EntityMeta, RawBlock, WriterConfig, assemble};
use pvec_container::{Bounds, Header, LengthUnit, Manifest, Metadata};
use pvec_entity::EntityId;

/// One raster: its placement, the tiles of its pyramid (an empty `bytes` is a tile that is not stored) and how
/// the index lays them out.
pub struct RasterIn {
    /// The placement written to the stream; the raster's id is its position in the slice.
    pub placement: Placement,
    /// Every tile of every level, as `blocks::raster::build` takes them.
    pub tiles: Vec<TileIn>,
    /// How many of the coarsest levels go inline in the index block.
    pub inline_levels: u32,
    /// The most bytes of a chunk.
    pub chunk_bytes: u32,
}

/// A writer with the one line of geometry the artifact needs (WRT-01 refuses a file with no entity).
pub fn line_writer() -> ContainerWriter {
    let mut writer = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "pvec-testkit".into(),
            producer_version: "0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    let id = EntityId::new(0).expect("id 0");
    let bounds = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
    writer.push_line(EntityMeta { id, style: 0, cell: 0, bounds }, [0.0, 0.0], [1.0, 1.0]).expect("a line");
    writer
}

/// `writer`, finished with the raster stream pushed to it, and the raster blocks assembled after its geometry
/// in the canonical order: every kind 9 block by raster, then every kind 10 block by raster and subtype. Sealed
/// with the artifact digest.
pub fn raster_file(mut writer: ContainerWriter, rasters: &[RasterIn], max_tiles: u64) -> Vec<u8> {
    let placements: Vec<Placement> = rasters.iter().map(|r| r.placement.clone()).collect();
    let version = pvec_capabilities::Registry::v1().get(RASTER_CAPABILITY).expect("capability 21").version;
    let payload = encode_rasters(&placements).expect("the placements encode");
    writer.push_stream(Stream { capability: RASTER_CAPABILITY, version, payload }).expect("the stream");
    let base = writer.finish().expect("the geometry writes");

    let len = base.len() as u64;
    let header = Header::parse(&base[..base.len().min(4096)], len).expect("a header");
    let (m0, m1) =
        (header.manifest_offset as usize, (header.manifest_offset + header.manifest_length) as usize);
    let manifest = Manifest::parse(&base[m0..m1], &header, len).expect("a manifest");
    let d = manifest.directories[0];
    let entries = parse_directory(
        &base[d.offset as usize..(d.offset + d.length) as usize],
        u64::from(header.header_len),
        len,
    )
    .expect("a directory");
    let mut blocks: Vec<RawBlock> = entries
        .iter()
        .map(|e| RawBlock {
            kind: e.kind,
            subtype: e.subtype,
            cell: e.cell,
            count: e.count,
            bounds: e.bounds,
            flags: e.flags,
            codec: e.codec,
            capability: e.capability,
            uncompressed_length: e.uncompressed_length,
            stored: base[e.offset as usize..e.offset as usize + e.stored_length as usize].to_vec(),
        })
        .collect();

    let mut raster_blocks = Vec::new();
    for (cell, r) in rasters.iter().enumerate() {
        let p = &r.placement;
        let grid: Vec<(u32, u32)> = (0..u32::from(p.levels))
            .map(|l| grid_of(p.width, p.height, u32::from(p.tile_edge), u32::from(p.levels), l))
            .collect();
        let built = build(&grid, r.tiles.clone(), r.inline_levels, r.chunk_bytes, max_tiles)
            .expect("the tiles build");
        let [x0, y0, x1, y1] = p.visible_box.map(f64::from);
        let [a, b, c, dd, tx, ty] = p.pixel_to_source;
        let corners =
            [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(u, v)| [a * u + b * v + tx, c * u + dd * v + ty]);
        let fold = |f: fn(f64, f64) -> f64, i: usize| corners.iter().map(|q| q[i]).fold(corners[0][i], f);
        let footprint = Some(Bounds {
            min: [fold(f64::min, 0), fold(f64::min, 1)],
            max: [fold(f64::max, 0), fold(f64::max, 1)],
        });
        let raw = |kind, st, count, stored: Vec<u8>| RawBlock {
            kind,
            subtype: st,
            cell: cell as u32,
            count,
            bounds: footprint,
            flags: FLAG_HAS_BOUNDS,
            codec: 0,
            capability: CAPABILITY,
            uncompressed_length: stored.len() as u32,
            stored,
        };
        raster_blocks.push(raw(KIND_INDEX, 0, built.index.stored(), built.index.encode()));
        for ch in built.chunks {
            raster_blocks.push(raw(
                KIND_TILES,
                subtype(ch.level, ch.chunk).expect("a subtype"),
                ch.count,
                ch.bytes,
            ));
        }
    }
    raster_blocks.sort_by_key(|b| (b.kind, b.cell, b.subtype));
    blocks.extend(raster_blocks);
    assemble(manifest, &blocks, Integrity::Full).expect("the file assembles")
}
