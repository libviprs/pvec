# ADR 0019: Calibration (scale and georeference) inside a PVec artifact

Status: Accepted, 2026-11-07 (the design panel on #263, recorded in its comment of that date). Decides #263's
georeference and scale questions; the viewer question of #263 is not decided here. Builds on ADR 0002 (authored
bits), ADR 0006 (per-block checksum), ADR 0008 and ADR 0010 (extension rules, the registry) and ADR 0018 (the
`Raster` class and capability-owned block kinds, whose open question 2 this closes). The byte layout is
`docs/CALIBRATION.md`.

I ran no benchmark for this ADR. Every number below is a constant or a document I read, or plain arithmetic on
them, and the one I could only estimate says so.

## Context

A PVec is in source space. The manifest's `metres_per_unit` says what one coordinate unit is, from the drawing
header (`docs/CONTAINER.md` chapter 1). Three things have no home:

* a PDF is in points, which the unit table cannot name, so the manifest says 1.0 and the page has no real-world
  scale at all (libviprs `src/pvec_convert/pdf.rs`);
* nothing lets someone calibrate a drawing afterwards (two measured points and a distance, or a plan scale such as
  1:100) and have it persist in the file;
* the georeference lives in a libviprs `.georef.json` sidecar that holds an opaque `calibration_id` and source-only
  control points with no world coordinate, so it cannot become a georeference by itself.

The owner settled three things before the panel: the georeference goes inside the `.pvec`, a scale calibration
persists in the file, and MapLibre is not the viewer. The constraints the container puts on the answer:
`CONTAINER.md` 1.3 and 1.6 (declaring one more capability grows the manifest by exactly 16 bytes, and a non-zero
reserved field is refused by every 1.0 reader), 4.3 (the canonical order), 4.7 (XXH3-64 per block over the stored
bytes, SHA-256 over the whole file), 5.5 (`assemble` lays out an artifact from encoded blocks byte for byte) and
`docs/READER.md` LAYOUT-03 (bytes after the directory are refused).

## Options

1. **One optional capability owning one block kind, rewritten through `assemble` (chosen).** Capability 22, kind
   11, one block, always last. A replace rewrites the calibration block, the checksum table (8 bytes a block) and the
   directory (64 bytes a block): for the 1,954 block million entity fixture of ADR 0006 that is about 141 KB plus the
   block (arithmetic). The SHA-256 is recomputed, one whole-file read: 65.8 MB at 2,467 MB/s native x64 is 27 ms, 130
   ms in wasm (ADR 0006's measured rates; a 1 GB scan-heavy file would be about 0.4 s, estimate).
2. **A stream in the component set (kind 4).** Kind 4 sits before the spatial directory, the reference arrays, the
   straddlers, Tier 0 and every raster block, so a rewrite that changes its length moves all of them. Rejected.
3. **An appendable trailing block.** The capability entry still has to enter the manifest, and LAYOUT-03 forbids
   bytes after the directory, so the tail is rewritten anyway and nothing is saved. Rejected.
4. **A region in the header or manifest reserved space.** Needs non-zero reserved fields, which every 1.0 reader
   refuses. Rejected.
5. **Two capabilities, scale and georeference.** Two negotiation rows, two effects, two sets of exports and two
   fuzz targets for one feature, and two blocks that can disagree. Rejected.
6. **Declaring 22 in every file to avoid the 16 byte shift.** Every 1.0.0 and 1.1.0 reader would then report a
   capability it cannot show on files that carry nothing. Rejected.

## Decision

**Capability and class.** One capability, 22 `calibration`, under a new registry class `Calibration` (the ADR 0010
amendment of 2026-11-07): optional by construction and never required, refers to no entity, may own block kinds. It
owns one block kind, 11 `calibration`, one block per artifact, sorted after every other kind so it is always the
last block. Not a reuse of 21 (a scale must not depend on a raster) nor of 19 (a component is entity-keyed). Scale
and georeference are two optional records of the one block, at least one present, because one operation rewrites
them and one consumer reads them. NEGOTIATE-04 is worded by class: a capability whose class is never required.

**Granularity.** One per artifact. A raster's world position is `source_to_crs` after `pixel_to_source`, one
composition, so a per-raster georeference would bring back the "two georeferences in one file could disagree" case
ADR 0018 refused. This answers ADR 0018 open question 2: the CRS identifier lives in the artifact's calibration block.

**The georeference record.** An affine source to CRS in the raster's `[a b c d tx ty]` order, held to NP-19; a CRS
kind (projected, geographic, engineering); a method (declared, fitted, manual); a CRS id as an authority code of at
most 64 bytes; an optional opaque WKT2 or PROJJSON definition of at most 16 KiB that no PVec crate parses; 0 to
1,024 control points as evidence; and the fit's maximum residual. The affine is authored and mandatory: a
reader-side fit gives implementation-dependent bits, and PVec's numeric story is authored bits and one rule. The
points stay so the fit can be audited and redone after a CRS correction.

**The scale record.** `metres_per_source_unit` with a provenance (declared unit, plan scale, measured, from the
georeference, manual), the evidence that provenance needs, an optional relative uncertainty and a note. The block
also carries one `external_id`. The manifest unit is never touched. Disagreement between the two is the normal case,
not a conflict: the reader exposes both, `effective_scale()` returns the calibration with its provenance when there
is one and the manifest otherwise, and nothing is applied to coordinates.

**Re-calibration.** A canonical rewrite through `assemble` with every other block raw-copied, to a temporary file
then renamed. Block bytes and XXH3 sums never change; on a replace no other block moves; on the first add and on a
remove every offset moves by 16. A rewrite equals a one-shot write byte for byte, held by a test, so a file never
has two byte forms for one content. A reader with a stale directory fails loudly on the block magic or
INTEGRITY-03; `If-Range` with a captured ETag is the proper fix over HTTP and is pvec-sources' follow-up.

**Versioning.** Container stays 1.0. `FORMAT_VERSION` goes to 1.2.0. Entry 22 is at version 1.0.0, optional,
`min_reader` 1.2.0, `status_since` 1.2.0. The `min_reader` hoist covers required capabilities only, so a 1.0.0 or
1.1.0 reader opens a calibrated file `OpenDegraded` with 22 under `SkipReason::NotImplemented` and the new
`Effect::Uncalibrated`, the kind 11 entry listed as `UnknownBlockKind` naming 22, the payload never fetched. Golden
files without a calibration are byte-identical before and after. The WASM ABI stays 2 with additive exports.

**Limits and hostile input.** LIMIT-17 `max_calibration_bytes` = 65,536 in both presets, checked from the
directory entry before the block is fetched, and the fixed `CAL-01` to `CAL-12` rules of `docs/CALIBRATION.md`.
Each refuses the calibration, never the file. A projective map is not representable; producers refuse it before
they get here. A twelfth fuzz target, `fuzz_calibration`.

### The choices the panel left to the implementer

* **Field order.** A 16 byte header (`PVCL`, version 1, flags, the payload length, the external id's length), then
  the 80 byte scale record and the 72 byte georeference record when their flag is set, then the variable tail:
  control points, CRS id, definition, note, external id. Every fixed part is a multiple of 8, so the points are
  aligned, and every length is in a fixed part, so the tail is sized before it is read. Evidence a provenance does
  not use is stored as zero rather than left out, so the scale record has one size and a reader needs no table.
* **The residual check in `validate`.** `ex = a*sx + b*sy + tx - wx`, `ey = c*sx + d*sy + ty - wy` left to right
  with no fused multiply-add, then `detmath::hypot(ex, ey)` (one correctly rounded `sqrt`, no libm), the largest over
  the points, compared with the stored residual by NP-05 at the scale of the largest world coordinate. It is CAL-11
  and runs in `pvec validate` only, never at open.
* **The WASM export names.** `calibration_stat(file, which)` for the integers (it doubles as the probe: a module
  that exports it has the group), `calibration_number(file, which)` for the floats, `calibration_point(file, i,
  which)` for a control point coordinate and `calibration_text(file, which)` for the CRS id, the definition, the note
  and the external id. The `which` tables are in `docs/WASM_ABI.md`.
* **LIMIT-17 is a `Limits` field**, `max_calibration_bytes`, so a host can lower it. Both presets hold 65,536, which
  is above the 49,894 byte largest block the rules allow.

## Consequences

* Every artifact written without a calibration is byte for byte what it was at 1.1.0; the golden digests do not
  move.
* A calibrated file has one more manifest entry and one more block, at the end. An old reader opens it degraded and
  says so.
* libviprs writes the calibration at conversion (libviprs#1543), stops writing the sidecar and keeps a one-version
  read-only importer for it.
* A re-calibration reads the whole file once to recompute the digest. Clearing the digest instead would be a silent
  downgrade.

## Dissents recorded by the panel

1. No definition text in v1, authority codes only. The majority keeps the capped opaque text because a GeoPDF whose
   `/GCS` is a `/WKT` exists today and the alternative is a sidecar again.
2. Refuse geographic targets. The majority allows them with CRS kind 2 and an explicit scale record.
3. Full LGIDict support in the first PDF slice (libviprs). The majority limits it to the no-table case.
4. A second, lazily read block for the points and the definition. The majority keeps one block under 64 KiB.

## Reversal threshold

Split the points and the definition into a second block if a real producer writes calibrations where the block read
at open costs more than 1 extra range or more than 64 KiB on 1 in 10 files of the corpus. Revisit the opaque
definition if a host ever needs PVec to interpret it, or if 2 independent producers disagree on what a WKT2 string
means for the same file.

## Staged plan

1. This ADR, `docs/CALIBRATION.md`, registry entry 22 and the class, kind 11 reserved, `Effect::Uncalibrated`,
   format 1.2.0, the compatibility row and matrix cases (#265).
2. The block parser and encoder, LIMIT-17, `fuzz_calibration` (#266).
3. Writer, reader, negotiation, facade accessors, `pvec info` and `validate` (#267).
4. `recalibrate` through `assemble`, and `pvec calibrate` (#268).
5. The WASM exports (#269).
6. `If-Range` with a captured ETag in pvec-sources.
