//! A large raster `.pvec` from a seed (issue #251), for the consumers that put a heap and a GPU budget under
//! load (maplibre-pvec example 04).
//!
//! The raster is the tiler's pyramid of stored-deflate PNGs, so its bytes depend on the shape alone. The
//! seed draws the linework on top: line segments and a few curves (cubic Beziers as B-splines, and circle
//! arcs) at random places over the raster, so strokes and refinement have something to do. The generator is
//! a splitmix64 in this file and every coordinate is a multiple of a quarter pixel, so the same arguments
//! give the same bytes on any machine.
//!
//! Sizes: a stored tile is about `tile_edge^2` bytes at 8 bits (an eighth of that at 1 bit), and the
//! pyramid adds a third. 8192 by 8192 with tiles of 1024 is 85 tiles and about 90 MB at 8 bits gray, about
//! 11 MB at 1 bit.

use crate::tiler::{Shape, tiles};
use pvec_capabilities::Version;
use pvec_components::{
    Band, PixelFormat, Placement, RASTER_CAPABILITY, RasterId, Stream, TileCodec, encode_rasters,
};
use pvec_container::Bounds;
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{LengthUnit, Metadata};
use pvec_entity::EntityId;

/// One `.pvec` with a raster of `shape` (the tiler's pyramid, upright at one source unit a pixel) and `lines`
/// seeded line segments and `curves` seeded curves drawn over it, in pixel units.
pub fn large_raster(seed: u64, shape: Shape, lines: u32, curves: u32) -> Vec<u8> {
    let mut state = seed;
    let mut next = move |bound: u32| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // a quarter-pixel position in 0..bound
        f64::from((z % (u64::from(bound) * 4)) as u32) / 4.0
    };
    let (w, h) = (shape.width, shape.height);
    let mut writer = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-testkit".into(),
            producer_version: "0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    let boxed = |p: &[[f64; 2]]| Bounds {
        min: [
            p.iter().map(|q| q[0]).fold(f64::MAX, f64::min),
            p.iter().map(|q| q[1]).fold(f64::MAX, f64::min),
        ],
        max: [
            p.iter().map(|q| q[0]).fold(f64::MIN, f64::max),
            p.iter().map(|q| q[1]).fold(f64::MIN, f64::max),
        ],
    };
    for id in 0..lines + curves {
        let meta =
            |bounds| EntityMeta { id: EntityId::new(id).expect("an id"), style: id % 4, cell: 0, bounds };
        if id < lines {
            let p = [[next(w), next(h)], [next(w), next(h)]];
            writer.push_line(meta(boxed(&p)), p[0], p[1]).expect("a line");
        } else if id % 2 == 0 {
            let p = [[next(w), next(h)], [next(w), next(h)], [next(w), next(h)], [next(w), next(h)]];
            let spec = SplineSpec {
                degree: 3,
                knots: &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                x: &p.map(|q| q[0]),
                y: &p.map(|q| q[1]),
                weights: None,
                closed: false,
                periodic: false,
            };
            writer.push_spline(meta(boxed(&p)), spec).expect("a spline");
        } else {
            let (c, r) = ([next(w), next(h)], next(w.min(h) / 2 + 1) + 1.0);
            let bounds = boxed(&[[c[0] - r, c[1] - r], [c[0] + r, c[1] + r]]);
            let start_sweep = [next(7), next(7) - 3.5];
            writer.push_circle_arc(meta(bounds), c, r, start_sweep).expect("an arc");
        }
    }
    let placement = Placement {
        pixel_to_source: [1.0, 0.0, 0.0, -1.0, 0.0, f64::from(h)],
        width: w,
        height: h,
        visible_box: [0, 0, w, h],
        tile_edge: shape.tile_edge as u16,
        levels: shape.levels() as u8,
        tile_codec: TileCodec::Png,
        pixel_format: match shape.channels {
            1 => PixelFormat::Gray,
            2 => PixelFormat::GrayAlpha,
            3 => PixelFormat::Rgb,
            _ => PixelFormat::Rgba,
        },
        bit_depth: shape.bit_depth,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    };
    writer
        .push_stream(Stream {
            capability: RASTER_CAPABILITY,
            version: Version::new(1, 1, 0),
            payload: encode_rasters(std::slice::from_ref(&placement)).expect("the placement encodes"),
        })
        .expect("the stream");
    writer
        .push_raster_tiles(RasterId::new(0).expect("raster 0"), tiles(shape, |_, _, _| true))
        .expect("the tiles");
    writer.finish().expect("the file writes")
}
