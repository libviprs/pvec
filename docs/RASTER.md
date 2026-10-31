# Rasters inside a PVec artifact

This is the byte-level specification of ADR 0018 (`docs/adr/0018-raster-in-pvec.md`, Accepted 2026-10-31). The ADR
says why; this page says what is written. A raster is a placement in a stream of capability 21 `raster` plus two
block kinds the capability owns, 9 `raster_index` and 10 `raster_tiles` (`docs/CONTAINER.md` chapter 4). PVec stores
the encoded tile bytes it was given and never encodes or decodes a pixel. Capability 21 is optional and can never be
required, the container stays 1.0 and the format version is 1.1.0, so a 1.0.0 reader opens the file degraded and
skips the stream and both kinds. This page is the layout; the code that writes and reads it arrives with #217 (the
stream), #218 (the blocks) and #219 (the sniffers), and the interchange chapter with #222.

All integers and floats are little endian. A reserved field is zero and a reader refuses a file where it is not.

## 1. The placement

The stream is a `u32` count `n`, then `n` records one after the other. The `RasterId` of a record is its position,
dense from 0, in the producer's source draw order. A record is 88 bytes and then its variable tail:

| offset | size | field | type | meaning |
|---|---|---|---|---|
| 0 | 48 | `pixel_to_source` | 6 x f64 | `[a b c d tx ty]`, authored bits, see 1.1 |
| 48 | 4 | `width` | u32 | pixels of the full resolution level, at least 1 |
| 52 | 4 | `height` | u32 | same, at least 1 |
| 56 | 16 | `visible_box` | 4 x u32 | `x0 y0 x1 y1`, `x0` and `y0` inclusive, `x1` and `y1` exclusive, inside `width` by `height` and non-empty |
| 72 | 2 | `tile_edge` | u16 | 256, 512 or 1024 |
| 74 | 1 | `levels` | u8 | at least 1; level `levels - 1` is full resolution |
| 75 | 1 | `tile_codec` | u8 | 1 PNG, 2 JPEG, 3 WebP; an unknown id skips the raster and reports it |
| 76 | 1 | `pixel_format` | u8 | 0 gray, 1 gray_alpha, 2 rgb, 3 rgba |
| 77 | 1 | `bit_depth` | u8 | 1 or 8 |
| 78 | 1 | `opacity` | u8 | 255 is opaque, multiplied into the fragment alpha |
| 79 | 1 | `band` | u8 | 0 underlay (below hatch fills and every stroke), 1 overlay (above both) |
| 80 | 4 | `layer_index` | u32 | a row of the layer stream's name table, or `0xFFFFFFFF` for none; ignored if that stream is skipped |
| 84 | 1 | `digest_len` | u8 | 0 or 32 |
| 85 | 1 | `attribution_len` | u8 | 0 to 255 |
| 86 | 2 | reserved | u16 | zero |
| 88 | `digest_len` | `source_digest` | bytes | SHA-256 of the source raster, provenance only |
| next | `attribution_len` | `attribution` | UTF-8 | plain text, RASTER-07 |

### 1.1 `pixel_to_source`

The map is `x = a*px + b*py + tx` and `y = c*px + d*py + ty`, in PVec's `Transform2` order `[a b c d tx ty]`. The
pixel origin is the top-left corner of pixel (0, 0), pixel y points down (GDAL `PixelIsArea`; a world file or a
`PixelIsPoint` GeoTIFF is shifted by half a pixel first). The target is the same source space as the vectors, which
is y-up, so an upright image has `d < 0` and a negative determinant. That is the normal case: a renderer must not
cull by winding. A reflection or a non-uniform scale is legal, a singular map is refused (RASTER-05). The writer
normalises `-0.0` to `+0.0` before the bits are compared.

### 1.2 The pyramid

Level `L` has width `w_L` and height `h_L`, with `w_(levels-1) = width` and each level above halving with a
ceiling, `w_(L-1) = ceil(w_L / 2)`. The pyramid stops at the first level that is one tile. The grid is anchored at
pixel (0, 0) with `cols = ceil(w_L / tile_edge)` and `rows = ceil(h_L / tile_edge)`, no overlap, and `cols` and
`rows` are at most 65,535. A tile is a full `tile_edge` square: an edge tile is padded by the producer and the
renderer clips to `visible_box`, so a padded margin never draws.

## 2. The index block (kind 9)

One per raster. Directory entry: capability 21, `cell` = the `RasterId`, `subtype` 0, `count` = tiles stored,
bounds = the axis-aligned box of the `visible_box` footprint in source space with the bounds flag set, codec 0.
A reader filters by kind before cell. The payload, in this order:

| part | size | content |
|---|---|---|
| header | 32 | `PVX1` (`50 56 58 31`), `version` u16 = 1, `levels` u16, `tile_total` u32 (every entry, stored or not), `tile_stored` u32, `chunk_total` u32, `inline_len` u32, 8 reserved bytes |
| level table | 16 per level, level 0 first | `cols` u32, `rows` u32, `chunks` u32, `first` u32 (the ordinal of the level's first tile entry) |
| tile table | 24 per tile | `chunk` u32, `offset` u32, `length` u32, `col` u16, `row` u16, `xxh3` u64 |
| inline part | `inline_len`, starting at an 8 byte boundary | the bytes of the inline tiles, one after the other |

Within a level the tile table is in **Hilbert order**: the curve over the smallest power-of-two square that holds
`cols` by `rows`, standard `d2xy` with the first step along x, cells outside the grid skipped. The entry carries its
own `col` and `row`, so a reader needs no inverse curve. `chunk` is the chunk of the level that holds the tile, or
`0xFFFFFFFF` for a tile in the inline part, and `offset` is a byte offset inside that chunk or inside the inline
part. `length` 0 means the tile is not stored and draws nothing (a sparse pyramid); `xxh3` is the XXH3-64 of the
tile's bytes. The coarsest levels, in order, are stored inline up to the Tier 0 cap (ADR 0018, "Tier assignment"),
so the first paint of every raster is one range over the adjacent kind 9 blocks. There is no `tier0_level` field:
the reader sees what is inline.

## 3. The tile blocks (kind 10)

One per (raster, level, chunk). Directory entry: capability 21, `cell` = the `RasterId`,
`subtype = level * 1024 + chunk` (unique, and ascending by level under LAYOUT-04), `count` = tiles in the chunk,
bounds = the box of the chunk's tiles, codec 0 (PNG, JPEG and WebP are compressed already). The payload is the
tiles' encoded bytes one after the other in the order of the tile table, with no header: the index delimits them.
A chunk never exceeds `max_block_bytes`, a level that would is split, and `chunk` is below 1024. A tile read is a
range inside the chunk at the index's offset, checked against the index's XXH3-64 before the bytes leave the
reader. The chunk's whole-block checksum is checked by full validation only, as the straddler pages are.

## 4. Conversions into `pixel_to_source`

Each conversion gives the six numbers in `[a b c d tx ty]` order for an image `w` pixels wide and `h` high. A
producer that starts from a map into some other space (geographic, page) composes it with the map from that space
into source space, folding in the metres-per-unit factor, and refuses the composition when its precondition fails
(ADR 0018, "Georeference").

**libviprs `GeoTransform`.** Its `[a b c d e f]` is `geo_x = a*px + b*py + c` and `geo_y = d*px + e*py + f` with the
same pixel convention, so the PVec order is `[a b d e c f]`.

**maplibre-pvec SVG-order calibration.** The calibration is `matrix(a b c d e f)`, `x' = a*x + c*y + e` and
`y' = b*x + d*y + f`. Read as a map in PVec order it is `[a c b d e f]`, so a placement in the calibration's target
space is composed with the inverse of the drawing's calibration to land in source space.

**DXF `IMAGE`.** With the insertion point `P` (the lower-left corner of the image), the pixel vectors `U` (along
the bottom edge) and `V` (up the left edge): `[Ux -Vx Uy -Vy Px + h*Vx Py + h*Vy]`. The clip boundary is in
pixel-centre coordinates, `(-0.5, -0.5)` to `(w - 0.5, h - 0.5)` for the whole image, with rows counted from the
bottom, so a boundary point `(u, v)` is the pixel-corner point `(u + 0.5, h - (v + 0.5))`: half a pixel and a row
flip. The producer bakes the clip into alpha and sets `visible_box` to its bounds.

**PDF CTM.** An image XObject fills the unit square, with its first row at the top, under `CTM = [A B C D E F]`
(`x = A*u + C*v + E`, `y = B*u + D*v + F`). Pixel `(px, py)` is `(u, v) = (px / w, 1 - py / h)`, which gives
`[A/w -C/h B/w -D/h C+E D+F]` in page space, to be composed with the page to source map.

## 5. Test vector

A rotated and sheared placement of a 1024 by 512 image, every number exact in binary so the corners are exact:

| field | value |
|---|---|
| `pixel_to_source` | `[0.5 0.25 0.125 -0.5 100 50]` |
| `width`, `height`, `visible_box` | 1024, 512, `0 0 1024 512` |
| determinant | `0.5 * -0.5 - 0.25 * 0.125 = -0.28125` (negative, as for any upright image) |

The expected corners, in source space:

| pixel corner | source point |
|---|---|
| (0, 0), top-left | (100, 50) |
| (1024, 0), top-right | (612, 178) |
| (1024, 512), bottom-right | (740, -78) |
| (0, 512), bottom-left | (228, -206) |

The centre of pixel (0, 0), the point (0.5, 0.5), is (100.375, 49.8125), and the box of the footprint is
x 100 to 740 and y -206 to 178. The same placement in the other orders: `GeoTransform` `[0.5 0.25 100 0.125 -0.5 50]`,
SVG order `[0.5 0.125 0.25 -0.5 100 50]`, DXF `P = (228, -206)`, `U = (0.5, 0.125)`, `V = (-0.25, 0.5)`, and PDF
`CTM = [512 128 -128 256 228 -206]`.

## 6. Rules

The rules refuse the raster, never the file. The limits LIMIT-13 to LIMIT-16 (`docs/SECURITY_LIMITS.md`) bound the
number of placements, the number of tiles, the stored bytes of one tile and the decoded bytes of a plan. The rendered
error table (`RasterError`, section 6.1) is generated from the code.

| rule | what is refused |
|---|---|
| RASTER-01 | `tile_edge` in {256, 512, 1024}; `levels >= 1` and `2^(levels-1) * tile_edge >= max(width, height)` with the pyramid stopping at the first one-tile level; `visible_box` inside `width` by `height` |
| RASTER-02 | per level, `cols * rows` computed in checked `u64` equals the index's entries for that level; chunk offsets ascending, non-overlapping and inside the chunk; the sum over levels equals the index `count` |
| RASTER-03 | the encoded header is read by a bounded sniffer and must say `tile_edge` by `tile_edge`: PNG `IHDR` with bit depth equal to the placement's (1 or 8), colour type 0, 2, 4 or 6 matching the channels, no interlace; JPEG SOF0, SOF1 or SOF2 reached within a 64 KiB segment walk, `Nf` 1 for gray and 3 for RGB, nothing else; WebP `VP8`, `VP8L` or `VP8X` dimensions, the `ANIM` flag refused, no channel check (the header has none). The stream ends at the tile's last byte (`IEND` last, `EOI` last, RIFF size equal to the length): no trailing payload |
| RASTER-04 | `(tile_codec, pixel_format)` pairs: JPEG only with gray or RGB; an unknown codec id skips the raster |
| RASTER-05 | `pixel_to_source` passes NP-19 with the 2^20 bound, the footprint corners pass NP-02, `-0.0` is normalised |
| RASTER-06 | consistency from the directory and the stream alone: one placement per `RasterId` named by any kind 9 or 10 block, one kind 9 block per placement, `subtype` levels below `levels`, a kind 9 or 10 entry with a capability other than 21 is refused |
| RASTER-07 | `attribution` plain text as defined above; `source_digest` 32 bytes |

### 6.1 The error table

Every `RasterError` of `pvec-components` with its rule, as the code reports it. The rows below are generated from
`RASTER_ERRORS` and a test (`crates/pvec-checks/tests/issue_217_raster_errors.rs`) fails if they differ. The stream-level
rules are RASTER-01, 04, 05 and 07; RASTER-02, 03 and 06 belong to the index and the sniffers (issues 218 and 219).
LIMIT-13 is the count of placements. A damaged payload keeps its `COMP-NN` rule and drops this stream only.

<!-- raster-errors:begin -->
| rule | error | meaning |
|---|---|---|
| LIMIT-13 | `TooMany` | more placements than the ceiling of 4,096, refused before the table is allocated |
| RASTER-01 | `BadGrid` | a tile edge outside 256, 512 and 1024, a level count other than the fewest whose coarsest level is one tile, a grid wider than 65,535 tiles, or a visible box that is empty or outside the image |
| RASTER-04 | `BadFormat` | JPEG with anything but gray or RGB at eight bits; an unknown codec id is not an error, it skips the raster |
| RASTER-05 | `BadTransform` | a pixel to source map that fails NP-19 (a non-finite entry, a linear entry above 2^20, a singular map under PVec's rule) or a footprint corner that fails NP-02 |
| RASTER-07 | `BadText` | an attribution over 255 bytes, not UTF-8, or with a control character, a line or paragraph separator or a bidi override; a digest that is not 32 bytes |
| COMP-NN | `Stream` | the payload is damaged: truncated (COMP-01), with trailing bytes (COMP-02), a reserved field set (COMP-12) or an undefined tag (COMP-11) |
<!-- raster-errors:end -->
