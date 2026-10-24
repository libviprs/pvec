# Geometry block encoding

This is the byte layout of the geometry payload that `crates/pvec-codec` reads and writes: the
type-batched, structure-of-arrays layout with an offset-table arena that ADR 0004 chose (layout B),
version 1. It is the payload only. The block framing, the directory entry, compression and the
checksum around it are phase 4 (P4.2 to P4.6). Nothing here is frozen until the v1 freeze (P8.7).

All integers and floats are little endian. Every column starts on a multiple of 8 bytes from the start
of the block and every padding byte is zero; a block with non-zero padding is refused, which keeps the
encoding canonical (decode then encode gives the same bytes).

## Header

| offset | size | field |
|---|---|---|
| 0 | 4 | magic `PVG1` |
| 4 | 2 | version, 1 |
| 6 | 2 | flags, must be 0 |
| 8 | 4 | total length of the block in bytes, must equal the bytes present |
| 12 | 4 | entity count, below the NP-18 limit of 2^28 |
| 16 | 2 | number of section kinds present |
| 18 | 6 | reserved, must be 0 |

The kind directory follows at offset 24, eight bytes per kind present: the kind id (u16), a reserved
u16 (0) and the count of entities of that kind (u32, at least 1). Kinds are strictly ascending, so a
kind appears once. The counts must add up to the entity count in the header.

The data starts right after the directory, which is always a multiple of 8 bytes long. For each kind
present, in ascending kind order, the columns below follow one after the other.

## Sections

`n` is the count of the kind. `ids` is `n` u32 entity ids (each below 2^28, unique in the block),
followed by padding to 8. Columns of `f64` are `n` values and need no padding. A `flags` column is `n`
bytes followed by padding; bit 0 is the only defined bit.

| id | name | columns, in order |
|---|---|---|
| 0 | `Line` | ids, origin x, origin y, direction x, direction y |
| 1 | `Circle` | ids, center x, center y, axis x, axis y, radius |
| 2 | `Ellipse` | ids, center x, center y, axis x, axis y, semi-axis 1, semi-axis 2 |
| 3 | `TrimmedLine` | ids, the four `Line` columns, trim start, trim span, flags (bit 0: reversed) |
| 4 | `TrimmedCircle` | ids, the five `Circle` columns, trim start, trim span, flags (bit 0: reversed) |
| 5 | `TrimmedEllipse` | ids, the six `Ellipse` columns, trim start, trim span, flags (bit 0: reversed) |
| 6 | `Polyline` | ids, vertex offsets (`n + 1` u32 padded), all x, all y |
| 7 | `Composite` | ids, flags (bit 0: declared closed), record offsets (`n + 1` u32 padded), record arena |
| 8 | `Bezier` | ids, record offsets (`n + 1` u32 padded), record arena of spline records |
| 9 | `BSpline` | ids, record offsets, record arena of spline records |
| 10 | `RationalBSpline` | ids, record offsets, record arena of spline records |
| 11 | `TrimmedSpline` | ids, record offsets, record arena of trimmed spline records |

The axis columns are the stored unit direction of the placement (or of the line), not an angle. The
decoder checks that it is unit length within the tolerance of NP-05 and keeps its bits; it does not
normalise it again, because normalising a unit vector can move its last bit.

The trim is the normalised trim of the numeric policy (NP-11, NP-12): `start` and `span` in the
parameter of the basis, increasing, plus the `reversed` flag. They are written as they are held and
rebuilt without re-normalising, so a round trip cannot move them.

`Polyline`: the offsets start at 0, never decrease, and every polyline has between 2 and 2^20
vertices. `x` and `y` each hold the total vertex count of the kind.

`Composite`: the offsets start at 0, are multiples of 8, never decrease and end at the length of the
arena. Each record is one composite and is consumed exactly. A record is a segment count (u32, between
1 and 2^16) and a zero u32, then that many segments. A segment is a tag (u32) followed by:

| tag | segment | payload |
|---|---|---|
| 1 | trimmed line | reversed (u32, 0 or 1), then the six `TrimmedLine` numbers |
| 2 | trimmed circle | reversed (u32), then the seven `TrimmedCircle` numbers |
| 3 | trimmed ellipse | reversed (u32), then the eight `TrimmedEllipse` numbers |
| 4 | polyline | vertex count (u32, between 2 and 2^20), then x and y interleaved |
| 5, 6, 7 | Bezier, B-spline, rational B-spline | a zero u32, then a spline record |
| 8, 9, 10 | trimmed Bezier, trimmed B-spline, trimmed rational B-spline | reversed (u32, 0 or 1), the trim start and span (f64), then the basis as a spline record |

A kind or tag that is not in the tables is refused. A curve kind with no codec yet is refused by the
encoder and never written as something else.

### Spline records

A spline record is four u32, then arrays of f64:

| word | meaning |
|---|---|
| 0 | degree. For a Bezier curve it must equal the control point count minus one |
| 1 | control point count: 2 to 65536 for a B-spline, 2 to 17 for a Bezier curve |
| 2 | knot count, at most 65553. Zero for a Bezier curve, which has no knot vector |
| 3 | flags, bit 0: declared closed. Any other bit is refused |

The control points follow as x and y interleaved, then the knots, then (rational only) one weight per
control point. Nothing about a record is believed before it is checked: the counts are compared with the
bytes left before the arrays are read, and the arrays are then handed to the constructors of
`pvec-geometry`, which refuse a knot vector whose length is not points plus degree plus one, one that
decreases, an interior knot repeated more than the degree times, a weight that is zero, negative, not
finite or below the conditioning floor, and a hull that collapses to a point. A rational curve whose
weights are all equal is still rational: the section says what it is and nothing is demoted.

A trimmed spline record (section 11) is the basis tag (u32: 1 Bezier, 2 B-spline, 3 rational), the
reversed flag (u32), the trim start and span (f64), then the basis as a spline record. The trim is
rebuilt from the written numbers and is not normalised again.

## Rules a decoder refuses on

Each refusal is a typed error with a rule identifier. `NP-nn` is the numeric policy; the geometry
constructors report their own rules for a curve they refuse.

| rule | meaning |
|---|---|
| ENC-01 | the block is shorter than its structure requires |
| ENC-02 | bad magic |
| ENC-03 | unsupported version |
| ENC-04 | a reserved field is not zero |
| ENC-05 | the declared length differs from the bytes present, or part of the block is unused |
| ENC-06 | the per-kind counts do not add up to the entity count |
| ENC-07 | the kind directory is not strictly ascending, or has an empty entry |
| ENC-08 | unknown kind id |
| ENC-09 | an entity id at or above the limit |
| ENC-10 | a repeated entity id |
| ENC-11 | an undefined flag bit |
| ENC-12 | a malformed offset table |
| ENC-13 | an unknown segment tag |
| ENC-14 | a malformed record |
| ENC-15 | non-zero padding |
| ENC-16 | (encoder) a curve kind with no codec |
| ENC-17 | (encoder) the block would not fit 32-bit offsets |

Every count is compared with the number of bytes left before it can size an allocation: a header that
claims 2^28 entities in a 40 byte block is refused as truncated without allocating for them.

## What is not here

Periodic (wrapped) B-splines have no form of their own, as the profile says; a closed curve written with
an unclamped knot vector and wrapped control points is an ordinary B-spline here, and a periodic spline given
as its unique points and one period of knots is unwrapped to the clamped form by `BSpline::from_periodic`
before it reaches the codec (ADR 0011, amendment for issue 93). Component references per
entity, which the issue text mentions, are deliberately not written: the component streams (P3.7)
refer to entities by `EntityId`, so the entity side carries no reference, and a reserved column would
cost four bytes per entity against the 36.7 bytes per line that ADR 0004 measured.
