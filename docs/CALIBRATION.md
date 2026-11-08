# Calibration inside a PVec artifact

This is the byte-level specification of ADR 0019 (`docs/adr/0019-calibration.md`). The ADR says why; this page says
what is written. A calibration is one block of kind 11 `calibration`, owned by capability 22 `calibration`
(`docs/CONTAINER.md` chapter 4), with two optional records in it: a **scale** (what one source unit stands for in
the world, and where that claim came from) and a **georeference** (an affine from source space into a coordinate
reference system, the CRS identifier, and the control points it came from). At least one is present. Capability 22
is optional and can never be required, the container stays 1.0 and the format version is 1.2.0, so a 1.0.0 or 1.1.0
reader opens the file degraded with `Effect::Uncalibrated` and never fetches the block.

All integers and floats are little endian. A reserved field is zero and a reader refuses the calibration where it is
not. Every float is stored with `-0.0` written as `+0.0`, so the bits compare equal after a round trip, and a stored
`-0.0` is refused (CAL-04). Nothing here is applied to coordinates: the geometry is in source space whether or not a
calibration is present, and a reader draws it the same way.

## 1. The block

One per artifact. Directory entry: kind 11, capability 22, `cell` 0, `subtype` 0, `count` = the number of records
present (1 or 2), must-understand clear, has-bounds clear, codec 0 (the default policy names 0 for kind 11, so a
rewrite tool needs no compressor; a reader still decodes another codec it knows). The writer puts it after every
other block, kind 9 and 10 included, so it is always the last block in the canonical order (`CONTAINER.md` 4.3). A
second kind 11 block, or one with `cell`, `subtype` or a flag set, is CAL-12, from the directory alone. Its stored
and decoded lengths are held to LIMIT-17 (`max_calibration_bytes`, 65,536) before a byte is fetched.

The payload is the 16 byte header, the fixed records the flags say are present (scale first, then georeference),
then the variable tail in this order: the control points, the CRS id, the CRS definition, the scale note and the
external id. Every fixed part is a multiple of 8 bytes, so the control points are 8 byte aligned. The payload is
consumed exactly.

### 1.1 Header

| offset | size | field | type | meaning |
|---|---|---|---|---|
| 0 | 4 | `magic` | 4 bytes | `PVCL` (`50 56 43 4C`) |
| 4 | 2 | `version` | u16 | 1 |
| 6 | 2 | `flags` | u16 | bit 0 a scale record follows, bit 1 a georeference record follows; the rest zero |
| 8 | 4 | `length` | u32 | the bytes of the whole payload, this header included |
| 12 | 1 | `external_id_len` | u8 | 0 to 255 |
| 13 | 3 | reserved | 3 x u8 | zero |

### 1.2 The scale record (80 bytes)

| offset | size | field | type | meaning |
|---|---|---|---|---|
| 0 | 8 | `metres_per_source_unit` | f64 | what one source unit stands for in the world, in `[2^-40, 2^40]` |
| 8 | 8 | `uncertainty` | f64 | relative, in `[0, 1)` when `has_uncertainty` is 1, `+0.0` otherwise |
| 16 | 8 | `paper_metres_per_unit` | f64 | `plan_scale` only: metres per source unit on the paper, in `[2^-40, 2^40]` |
| 24 | 4 | `ratio_numerator` | u32 | `plan_scale` only: the `1` of `1:100`, at least 1 |
| 28 | 4 | `ratio_denominator` | u32 | `plan_scale` only: the `100` of `1:100`, at least 1 |
| 32 | 16 | `p0` | 2 x f64 | `measured` only: the first source point, NP-01 and NP-02 |
| 48 | 16 | `p1` | 2 x f64 | `measured` only: the second source point |
| 64 | 8 | `distance_metres` | f64 | `measured` only: the real distance between them, in `[2^-40, 2^40]` |
| 72 | 1 | `provenance` | u8 | 1 `declared_unit`, 2 `plan_scale`, 3 `measured`, 4 `georeference`, 5 `manual` |
| 73 | 1 | `has_uncertainty` | u8 | 0 or 1 |
| 74 | 1 | `note_len` | u8 | 0 to 255 |
| 75 | 5 | reserved | 5 x u8 | zero |

A field a provenance does not use is zero (all bits clear), and one it uses is set (CAL-10):

| provenance | meaning | evidence |
|---|---|---|
| 1 `declared_unit` | the source declares a unit the manifest table cannot name, such as PDF points | none |
| 2 `plan_scale` | a ratio stated on a sheet | `ratio_numerator`, `ratio_denominator`, `paper_metres_per_unit` |
| 3 `measured` | two source points and the real distance between them | `p0`, `p1`, `distance_metres`; `p0` and `p1` further apart than NP-06's degenerate length (CAL-06) |
| 4 `georeference` | derived from the georeference record, which must be present | none |
| 5 `manual` | somebody typed it in | none |

PVec does not check that `metres_per_source_unit` agrees with its evidence: the evidence is there so a person or a
tool can audit it, and a producer may have rounded.

### 1.3 The georeference record (72 bytes)

| offset | size | field | type | meaning |
|---|---|---|---|---|
| 0 | 48 | `source_to_crs` | 6 x f64 | `[a b c d tx ty]`: `X = a*x + b*y + tx`, `Y = c*x + d*y + ty`, NP-19 |
| 48 | 8 | `residual_max` | f64 | in CRS units, `+0.0` unless `method` is 2 with at least 3 points |
| 56 | 4 | `definition_len` | u32 | 0 to 16,384 |
| 60 | 2 | `point_count` | u16 | 0 to 1,024 |
| 62 | 1 | `crs_kind` | u8 | 1 projected, 2 geographic, 3 engineering or local |
| 63 | 1 | `method` | u8 | 1 declared by the source, 2 fitted by the writer from the points, 3 manual |
| 64 | 1 | `crs_id_len` | u8 | 0 to 64 |
| 65 | 1 | `definition_format` | u8 | 0 none, 1 WKT2, 2 PROJJSON; 0 exactly when `definition_len` is 0 |
| 66 | 6 | reserved | 6 x u8 | zero |

The affine is in the raster's order (`docs/RASTER.md` 1.1), so one convention covers both. Its output is (easting,
northing) for a projected or engineering CRS and (longitude, latitude) for a geographic one, whatever axis order the
authority declares, so no reader has to look the authority up. A method 2 record has at least 2 points: a similarity
from 2, an affine from 3 or more, the count telling which.

At least one of the CRS id and the definition is present (CAL-09). The CRS id is an authority code,
`AUTHORITY:CODE`, of at most 64 bytes in `[A-Za-z0-9:_.+-]`, so `EPSG:27700`, `ESRI:102100`, `IAU_2015:30100` and
`urn:ogc:def:crs:EPSG::27700` all pass. The definition is opaque text of at most 16 KiB, tagged with its format, that
no PVec crate parses: it is handed to the host as bytes.

### 1.4 The tail

| field | size | type | meaning |
|---|---|---|---|
| control points | `point_count * 32` | `point_count` x 4 x f64 | `sx sy wx wy`: a source point (NP-01, NP-02) and its world point (finite, `|w| <= 2^40`) |
| `crs_id` | `crs_id_len` | ASCII | see 1.3 |
| `definition` | `definition_len` | UTF-8 | plain text; LF and TAB are the only controls allowed |
| `note` | `note_len` | UTF-8 | plain text, the scale's note (the `/R` string of a PDF `/Measure`, for example) |
| `external_id` | `external_id_len` | UTF-8 | plain text, the id an application keeps for this calibration (where the libviprs sidecar's `calibration_id` goes) |

The control points are evidence: the affine is authored, never refitted by a reader. Two points with the same source
point and different world points are refused (CAL-06); an exact duplicate is not. Text is held to the RASTER-07
rules: UTF-8, no C0 or C1 control, no U+2028 or U+2029, no bidi override or isolate (U+202A to U+202E, U+2066 to
U+2069), with LF and TAB allowed in the definition only.

The largest block the rules allow is 16 + 80 + 72 + 1,024 x 32 + 64 + 16,384 + 255 + 255 = 49,894 bytes, so LIMIT-17
never refuses a block the rules accept.

## 2. What a reader does with it

A reader without capability 22 skips the block from its directory entry and reports `Effect::Uncalibrated` ("this
file carries a scale or georeference your reader cannot show"); the geometry is bit-identical to the same file
without it. A reader with 22 reads the block at open, in one range, and checks it. A refused block refuses the
calibration and never the file: the linework opens and the report names the rule.

What a reader does with a CRS it does not know is nothing different from one it knows. PVec has no CRS database and
never reprojects (`docs/INTERCHANGE.md`): it hands the affine, the id, the definition bytes and the points to the
host, reports "georeference present, CRS `X`", and draws in source space as it always does. No guess, no fallback to
WGS84, no silent drop.

The manifest's `metres_per_unit` and the calibration's `metres_per_source_unit` answer two questions. The manifest
says what one coordinate unit *is* (a millimetre, from the drawing header); the calibration says what it *stands
for* in the world (0.1 m on a 1:100 sheet, 0.000352778 m for a PDF point). They differing is the normal case. The
manifest is never overwritten. The facade's `effective_scale()` returns the calibration's scale with its provenance
when there is one and the manifest's otherwise, never a bare number, and `pvec info` prints both.

A raster's world position is `source_to_crs` after its `pixel_to_source`: one composition, so a raster carries no
georeference of its own (this closes ADR 0018 open question 2).

## 3. Rules

<!-- calibration-errors:begin -->
| rule | error | meaning |
|---|---|---|
| LIMIT-17 | `TooLarge` | the block is stored as, or decodes to, more than `max_calibration_bytes` (65,536 by default and in the browser preset), refused from the directory entry before a byte is fetched |
| CAL-01 | `BadHeader` | the magic is not `PVCL`, the version is not 1, a flag bit other than 0 and 1 is set, or a reserved field is not zero |
| CAL-02 | `BadLength` | the `length` field is not the payload's length, the structure ends before the bytes do, or bytes are left over |
| CAL-03 | `TooMany` | more than 1,024 control points, a CRS id over 64 bytes or a definition over 16,384 bytes |
| CAL-04 | `BadNumber` | a float that is not finite, is subnormal, is `-0.0` or passes 2^40 (NP-01, NP-02); a factor or a distance outside `[2^-40, 2^40]`; an uncertainty outside `[0, 1)`; a negative residual |
| CAL-05 | `BadAffine` | the affine fails NP-19: singular under PVec's rule, or a linear entry above 2^20 |
| CAL-06 | `BadPoints` | two control points with the same source point and different world points, or a measured pair no further apart than NP-06's degenerate length |
| CAL-07 | `BadText` | a CRS id outside `[A-Za-z0-9:_.+-]`, or a definition, note or external id that is not UTF-8 or holds a control (LF and TAB in a definition aside), a line or paragraph separator or a bidi override |
| CAL-08 | `BadTag` | an undefined provenance, method, CRS kind or definition format, a `has_uncertainty` other than 0 or 1, or a definition format that disagrees with the definition's length |
| CAL-09 | `Empty` | neither record is present, or a georeference has neither a CRS id nor a definition |
| CAL-10 | `BadEvidence` | evidence set for a provenance that does not use it or missing for one that does, an uncertainty without its flag, a residual on a record that is not fitted from at least 3 points, a fitted record with fewer than 2 points, or provenance `georeference` without a georeference record |
| CAL-11 | `ResidualMismatch` | `pvec validate` only: the residual recomputed from the points and the affine is not NP-05 near the stored one at the scale of the largest world coordinate |
| CAL-12 | `Directory` | a second kind 11 block, or a kind 11 entry with `cell`, `subtype` or a flag set |
<!-- calibration-errors:end -->

A kind 11 entry that names a capability other than 22, or none, refuses the file at NEGOTIATE-06, like the raster
kinds. The writer refuses everything a reader would, as WRT-18 carrying the CAL rule.

### 3.1 The residual check (CAL-11)

`pvec validate` recomputes the residual with `+`, `-`, `*` and `detmath::hypot` only, so every platform gets the same
bits: for each point, `ex = a*sx + b*sy + tx - wx` and `ey = c*sx + d*sy + ty - wy`, evaluated left to right with no
fused multiply-add, then `hypot(ex, ey)`; the residual is the largest of those, and `+0.0` for no points. It is
compared with the stored `residual_max` by NP-05 (`Policy::near`) at the scale of the largest `|wx|` or `|wy|`. The
check runs only for method 2 with at least 3 points, since CAL-10 already holds every other record to a zero
residual. The open path does not run it: it is a validation of the producer's arithmetic, not of the bytes.

## 4. Re-calibration

A calibration is changed by rewriting the artifact through `assemble` with every other block copied raw, written to
a temporary file and renamed over the old one. Block bytes and their XXH3-64 never change. On a replace no other
block moves: the calibration block is last, so only it, the checksum table and the directory are new bytes. Adding
the first calibration, or removing the last, adds or drops the manifest's 16 byte capability entry, and every block
offset moves by 16. The artifact SHA-256 is recomputed. A rewrite gives exactly the bytes a one-shot write of the
same content gives, and a test holds it to that.

A reader that opened the old file and kept its directory meets a block whose magic or checksum fails, and reports
it; it never parses a stale block as a calibration.
