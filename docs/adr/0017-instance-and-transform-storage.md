# ADR 0017: Instance and transform storage

Status: Accepted, 2026-10-31 (proposed 2026-10-25). Decides #120. Ratified by Rom with ADR 0018, which amends it below. Builds on ADR 0001 (the profile is
for flattened drawings), ADR 0012 / #49 (what a transform does to a curve) and the no-silent-approximation
rule of #37.

## Context

Seven `r0-open` fixtures are refused by the canonical generator: `nested-blocks-001`,
`nonuniform-scale-circle-001`, `nonuniform-scale-rotated-001`, `reflection-001`, `rotation-irrational-001`,
`translation-001` and `uniform-scale-001`. Each keeps an instance plus a 3D matrix, and its structural
assertion `TRANSFORM_PRESERVED` says the matrix and its determinant survive as written. The container holds
neither an instance nor a transform: every record is `CURVE_ONLY`. #49 settled the geometric half (a
transform applied to a curve, and the promotion table). The storage half was never decided, so the generator
refuses the fixtures rather than emit the world-space curve, which would pass `WORLD_SPACE_RESULT` and fail
`TRANSFORM_PRESERVED`.

The charter in #1 already leans one way: `.pvec` is a container for flattened 2D drawings, deduplication is
a non-goal, a distinct DWG entity class is not a reason for a distinct type, and `libviprs` (EPIC N) maps
sources into the public semantic API. An INSERT is a source construct.

## Options

| question | options | evidence |
|---|---|---|
| what the artifact holds | A. curves only (curve-only): an instance is flattened to its world-space curves by the source mapping. B. an instance stream: a block table, a reference per instance and a 2x3 matrix stored as authored bits. C. the matrix as a component stream on the transformed curve | A is what every record is today and costs nothing new. B needs a block table, reference resolution with a depth limit and a cycle rule in every reader, and the viewer has to resolve instances to draw them (the spatial index of ADR 0005 is over world-space extents). `nested-blocks-001` is three levels deep, which C cannot express: a stream on a curve carries one matrix, not a chain of block references, so C only holds a single level and loses nesting |
| the structural assertion | A. `TRANSFORM_PRESERVED` stays in the corpus as a check on the container. B. it is a check on the source mapping (libviprs), and the corpus tags it so | under A the corpus would demand a capability the profile does not claim. Under B each of the seven fixtures keeps a geometric assertion that this repository can generate and check exactly |
| exactness | A. flatten with `Transform2` (ADR 0012). B. store both | ADR 0012 applies the matrix with no tessellation and refuses a singular map, so the world-space curve is derived, not approximated. A reflection or a non-uniform scale keeps its kind (a circle becomes an ellipse, #49) |

## Decision

Option A in the first row, B in the second, A in the third. PVec stays curve-only: an artifact holds no
block table, no instance and no transform. Flattening an instance is the source mapping's job (EPIC N),
done with `pvec-geometry`'s transforms, and the matrix and its determinant are checked there against the
source, not in the container.

For the seven fixtures: `WORLD_SPACE_RESULT` is generated from the world-space curve and checked exactly.
`TRANSFORM_PRESERVED` is re-tagged in `libviprs/pvec-gold-fixtures` as a source-mapping assertion. Until that
lands the generator keeps refusing them and its message cites this ADR.

## Consequences

* No change to the container, the capability registry or any reader. The seven refusals stay until the
  corpus is re-tagged, then they become seven generated fixtures.
* A consumer that wants to know a drawing used blocks cannot learn it from the artifact. That is the
  trade: the file is smaller to read and simpler to verify, and it is larger on disk than one that shares
  a block (deduplication is a non-goal in #1).
* Both follow-ups are done (#200): the corpus re-tagged `TRANSFORM_PRESERVED` as `SOURCE_MAPPING` (gold `1df9d1b`),
  and the generator emits the world-space curve for six of the seven when the record carries that assertion.
  `translation-001` is still refused: its world-space line is at z = 2 and the profile is 2D with no implicit
  promotion, which is a different gap from this ADR's.

## Amendment 2026-10-31: the curve-only rule is for geometry

ADR 0018 (raster graphics inside a PVec artifact) is the one exception to "PVec stays curve-only", and I say so here
without rewriting the Decision above. The rule stands for **geometry**: an artifact holds no instance, no block
table and no transform on a curve, and every entity is a curve. A raster placement is not an entity.

The exactness argument is why a transform is stored for pixels and not for curves. Flattening a curve through a
`Transform2` is exact (ADR 0012): the world-space curve is derived, not approximated, so storing the matrix buys
nothing. Pixels have no such flattening: resampling them into source space is lossy and cannot be undone, so ADR 0002's
authored-bits rule points the other way: keep the bytes and carry the 2x3 map. The other two reasons for refusing a transform still hold:
tiles stay out of the spatial index, and a viewer applies one affine per raster, never a tree of instances.

## Reversal threshold

Reopen this if a measured drawing is much larger flattened than as blocks (say more than 3x on the same
fixture on native x64) and the size matters to the streaming budget, or if the viewer needs to select or
edit a block as a unit. Then B is the option to take, with a depth limit and a cycle rule written first.
