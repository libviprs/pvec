# ADR 0021: A paint model for PDF fidelity, as four component streams

Status: Proposed, 2026-11-08. Tracks #286. The byte layout, the reader rules, the renderer and producer changes and the 45 regions it has to enable are in `docs/PAINT.md`. Builds on ADR 0014 (component streams), ADR 0018 (rasters) and ADR 0010 (the registry). Driven by planviewer issue 32 (Gitea, `iasbuilt/planviewer`).

I ran no benchmark for this ADR. The region list and its causes come from the fidelity spec in planviewer, and the sizes in the plan are my estimates.

## Context

PlanViewer compares a converted PDF page with PDFium region by region. With the fills, line styles and text it already reads, 45 regions still fail in its `vector-hatch` mode, and each failure is something the file cannot say: no fill rule, no paint order, no stroke colour, no gradient or tiling fill, no clip that is not a rectangle, alpha that the producer does not track, no text written, images only in a pyramid of the whole page.

Three facts shape the decision. The viewer already draws what the hatch, line style and text streams carry, and the fills it draws are exact where the PVec has them (a gap of 0 to 1 in 255). The format promises no paint order: entity ids are an order for storage and nothing says they are the order of painting. And a payload is consumed exactly (COMP-02) so a stream cannot grow a field in a minor version.

## Options

1. **Bump hatch, line style and text to new majors** carrying the fields. One stream each, nothing new in the registry. Every v1 reader then skips those streams by major version (COMP-20, `Skipped`), so a plan opens with no fills, no widths and no text in every reader that exists. The compatibility cost lands on exactly the readers I cannot update.
2. **Fields on the geometry entities** (a paint order and a colour per entity). Couples display to the geometry oracle, which ADR 0014 keeps apart, and makes every conformance verdict depend on style.
3. **New component streams beside the existing ones** (chosen). Four ids, each optional and skippable by length, and the existing streams unchanged, so the old readers keep what they have. The cost is more ids and a reader that wants the whole picture reads five streams where it could read two.
4. **Cut the PDF down in the producer**: bake clips into the geometry, resolve the fill rule into the rings, flatten gradients into bands of flat hatches, tiles into repeated entities. Needs no format change and works with today's viewer. It also destroys the authored vertices a snap or a measurement would use, turns a gradient into hundreds of entities, and makes a tiling pattern's size grow with its area. I keep it as the producer's fallback for a construct a stream cannot express, not as the design.

## Decision

Option 3: capabilities 23 `paint_order`, 24 `stroke_paint`, 25 `fill_paint` and 26 `clip`, class component, optional for good, version 1.0.0, format 1.3.0.

Choices inside it, with the alternative I turned down:

* **Paint order as ranges of items** (stroke by entity, fill by hatch record, raster, text), not a z number per entity. A z per entity costs four bytes a stroke for a million strokes where the ranges cost twelve bytes in all, and ranges cannot disagree with themselves. Rejected: a depth value in the geometry blocks, which would also have drawn translucent items in the wrong order whatever a viewer did.
* **A colour table with pairs, shaped like the line style stream**, and text coloured through its anchor entity. Rejected: a colour field in the text stream (a new major for text) and a colour inside the line style (the same, and it fuses two things that vary independently).
* **Fill rule, gradient and tiling in one stream keyed by hatch record**, with the flat colour left in the hatch record as the fallback. Rejected: a new hatch fill kind, which a v1 hatch reader refuses as COMP-11 and loses the whole stream for.
* **Gradients as sampled stops, never a PDF function.** The reader needs no PostScript calculator and the producer reports its error. Rejected: carrying the function, which is a security surface (a type 4 function is a program) for no region.
* **Tiling cells as primitives in the stream.** The one place a stream holds coordinates, argued in `docs/PAINT.md` 2.4 and bounded by COMP-27. Rejected: the cell as entities in a reserved space (they would be drawn in the page, enumerated, snapped to and indexed) and the cell as a raster (it would not scale with the zoom and the pyramid is exactly what a vector fill avoids).
* **Clips as rings of entities, applied at display**, with up to eight nested. Rejected: baking into geometry (option 4) and a clip stored as coordinates in the stream (a second way to write a curve).
* **Images through the existing raster capability.** The 12 raster and soft mask regions need the image in the PVec and ordered with the fills. Capability 21 has the placement, the soft mask as alpha and the clip as `visible_box`, and libviprs has the baker. The pyramid stays as the first picture. This changes the line in planviewer 32 that says the pyramid carries images by design, and I am saying so here on purpose.

## Consequences

* Every existing file opens exactly as before in every reader. Every new file opens in every existing reader, degraded and reported, drawing flat fills at the hatch colour, in the legacy order, unclipped.
* The registry grows by four, the format goes to 1.3.0, the compat matrix gets rows, `SECURITY_LIMITS.md` gets the ceilings of chapter 2 of `docs/PAINT.md`.
* A producer has to track more of a PDF's graphics state. The producer is the larger part of the work (seven PRs against three for the format).
* A viewer needs a fill tessellator that honours a rule, a batch list instead of a fixed stage order, a stencil for clips and two fill pipeline variants. The plan puts them in order of risk.
* Text, blend modes, groups, caps and joins are not decided here (PAINT.md chapter 10).

## Reversal threshold

If the batch count of real plans makes the ordered path slower than the legacy one beyond the ceiling in PAINT.md 5.3 on the corpus planviewer measures, I reopen the paint order and look at a depth-assisted scheme for opaque items. Nothing else in the design depends on how order is drawn.

## Staged plan

P1 to P3 in `docs/PAINT.md` chapter 8, each with red tests first. The streams are not required by anything until a producer writes them and a viewer reads them, so the order of landing is free.
