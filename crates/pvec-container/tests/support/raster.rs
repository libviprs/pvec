//! Raster fixtures for the writer tests (issue #218): placements, tiles of recognisable bytes, and a writer that
//! has one line of geometry and the raster stream.

use super::fixtures::{config, eid};
use pvec_components::{
    Band, PixelFormat, Placement, RASTER_CAPABILITY, RasterId, Stream, TileCodec, encode_rasters,
};
use pvec_container::blocks::raster::{TileIn, grid_of};
use pvec_container::writer::{ContainerWriter, EntityMeta};
use pvec_container::Bounds;

/// A placement of a `width` by `height` image in tiles of `edge`, upright in source space (`d < 0`) at one unit
/// a pixel, with the fewest levels whose coarsest is one tile.
pub fn placement(width: u32, height: u32, edge: u16) -> Placement {
    let mut levels = 1u8;
    while (u64::from(edge) << (levels - 1)) < u64::from(width.max(height)) {
        levels += 1;
    }
    Placement {
        pixel_to_source: [1.0, 0.0, 0.0, -1.0, 0.0, f64::from(height)],
        width,
        height,
        visible_box: [0, 0, width, height],
        tile_edge: edge,
        levels,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }
}

/// `(cols, rows)` of each level of a placement, coarsest first.
pub fn grid(p: &Placement) -> Vec<(u32, u32)> {
    let n = u32::from(p.levels);
    (0..n).map(|l| grid_of(p.width, p.height, u32::from(p.tile_edge), n, l)).collect()
}

/// Recognisable bytes for a tile: its place, repeated.
pub fn bytes_of(raster: u32, level: u32, col: u32, row: u32, len: usize) -> Vec<u8> {
    (0..len).map(|i| (raster * 97 + level * 31 + col * 7 + row * 3 + i as u32) as u8).collect()
}

/// Every tile of a placement's grid, `len` bytes each, except those `skip` names.
pub fn tiles(
    raster: u32,
    p: &Placement,
    len: usize,
    skip: impl Fn(u32, u32, u32) -> bool,
) -> Vec<TileIn> {
    let mut out = Vec::new();
    for (l, (cols, rows)) in grid(p).into_iter().enumerate() {
        for row in 0..rows {
            for col in 0..cols {
                let l = l as u32;
                let bytes = if skip(l, col, row) { Vec::new() } else { bytes_of(raster, l, col, row, len) };
                out.push(TileIn { level: l, col, row, bytes });
            }
        }
    }
    out
}

/// A writer with one line of geometry and the raster stream for `placements`.
pub fn writer_with(placements: &[Placement]) -> ContainerWriter {
    let mut w = ContainerWriter::new(config(&[], None));
    let bounds = Bounds { min: [0.0, 0.0], max: [10.0, 10.0] };
    w.push_line(EntityMeta { id: eid(0), style: 0, cell: 0, bounds }, [0.0, 0.0], [10.0, 10.0]).unwrap();
    w.push_stream(Stream {
        capability: RASTER_CAPABILITY,
        version: pvec_capabilities::Version::new(1, 0, 0),
        payload: encode_rasters(placements).expect("the placements encode"),
    })
    .unwrap();
    w
}

/// The pair of placements the raster tests share: 0 is 1000 by 600 (a 4 by 3 grid at full size, three levels),
/// 1 is 300 by 300 (two levels).
pub fn two_placements() -> Vec<Placement> {
    vec![placement(1000, 600, 256), placement(300, 300, 256)]
}

/// The artifact of the two rasters with every tile 700 bytes except the sparse ones of raster 0.
pub fn two_rasters_file() -> Vec<u8> {
    let ps = two_placements();
    let mut w = writer_with(&ps);
    w.push_raster_tiles(RasterId::new(0).unwrap(), tiles(0, &ps[0], 700, |l, c, r| l == 2 && (c + r) % 2 == 1))
        .unwrap();
    w.push_raster_tiles(RasterId::new(1).unwrap(), tiles(1, &ps[1], 700, |_, _, _| false)).unwrap();
    w.finish().unwrap()
}
