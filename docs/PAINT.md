# The paint model

This page is the byte-level design of ADR 0021 (`docs/adr/0021-paint-model.md`). The ADR says why, this page says what is written, what a reader does with it and what each producer has to change. The codecs for streams 23 to 26 are in `crates/pvec-components` (issue 286); the PDF mapper, the server and the PlanViewer renderer are still to come. It is the design they are built against, and the tracking issues are listed in chapter 9.

The driver is a PDF page that has to look in a PVec viewer like it does in PDFium. PlanViewer measures that region by region (planviewer issue 32): fills, clips, shadings, tiling patterns, alpha, images, soft masks and text. Forty-five of those regions cannot be drawn from a PVec today, and the reasons are all in what the file does not say, not in how a viewer draws what it is given.

All integers and floats are little endian. A reserved field is zero and a reader refuses a file where it is not. Framing, limits and refusal vocabulary are those of `docs/COMPONENT_STREAMS.md`; this page adds four streams to that set and does not touch the other seven.

## 1. What is missing, and where it goes

PVec already has most of the vocabulary. The hatch stream (17) carries a solid fill with an RGBA word, the line style stream (15, version 2) carries width, dash, phase and visibility, the text stream (16) carries anchored strings, and the raster capability (21) carries image placements with a band. The viewer reads all of those. What the file cannot say is below, and each row names the place it goes.

| gap | goes into | wire change |
|---|---|---|
| fill rule (nonzero or even-odd) | capability 25 `fill_paint` | new stream |
| paint order across fills, strokes, rasters and text | capability 23 `paint_order` | new stream |
| stroke colour (and text colour) | capability 24 `stroke_paint` | new stream |
| stroke width | line style 15, already there | none (a unit rule, section 6.3) |
| fill alpha and stroke alpha | the alpha byte of the RGBA words of 17 and 24 | none (the producer has to track it) |
| clips that are not rectangles | capability 26 `clip` | new stream |
| gradient fills for shadings | capability 25 `fill_paint` | new stream |
| tiling patterns | capability 25 `fill_paint` | new stream |
| text | text stream 16, already there | none (the producer has to write it) |
| hatch stream and a hidden line style | 17 and 15, already there | none (the server has to write them) |
| closed-ring fill cutting | the producer, not the format | none (section 6.4) |
| images and soft masks | capability 21, already there | none (the producer has to bake them, section 6.8) |

Four new capabilities, then, and no change to any stream that exists. That is deliberate (section 4).

## 2. The four streams

Each is a component stream: optional, never required, framed like the others, skipped by length by a reader that does not know it. Registry class `component`, version 1.0.0, min reader 1.0.0, so a 1.0.0 reader still opens the file. The format version goes from 1.2.0 to 1.3.0 because the registry grew.

| id | name | what it holds |
|---|---|---|
| 23 | `paint_order` | the order the items of the page were painted in |
| 24 | `stroke_paint` | a colour per entity |
| 25 | `fill_paint` | fill rule, gradient and tiling pattern fills, per hatch record |
| 26 | `clip` | clip regions made of entity rings, and which items they clip |

### 2.1 Paint items

Three of the four streams talk about the same four kinds of thing, so they share one vocabulary. A **paint item** is something a viewer draws as a unit:

| kind | item | numbered by |
|---|---|---|
| 1 | stroke | the `EntityId` of a visible entity |
| 2 | fill | the index of a record of the hatch stream (17), dense from 0 |
| 3 | raster | the `RasterId` of capability 21, dense from 0 |
| 4 | text | the index of a record of the text stream (16), dense from 0 |

A kind other than 1 to 4 is COMP-11. An item is always named by something that already exists in another stream or in the geometry, so these streams embed no curve and no pixel, exactly as the hatch stream does not.

### 2.2 Capability 23, `paint_order`

The payload is a `u32` run count (at most 1,048,576) and then that many 12 byte runs:

| offset | size | field |
|---|---|---|
| 0 | 4 | kind, 1 to 4 |
| 4 | 4 | first, the number of the first item |
| 8 | 4 | count, at least 1 |

A run stands for the items `first` to `first + count - 1` of that kind, in ascending order. The paint order is the runs in file order, and the items inside a run in ascending order. Paint order is back to front: the first item is painted first, so everything after it can cover it. The expanded list holds at most 2^24 items.

A producer that numbers its entities in the order it paints them gets a long run for the strokes and one run per change of kind, which is why a run is a range and not an item: a page with a million strokes and no fills costs 12 bytes.

Rules the decoder checks, besides the shape rules COMP-01 to COMP-05:

* **COMP-21**, an item named twice (in one run or across runs).
* **COMP-23** also covers the pattern transform of a tiling (it is inverted by the renderer just as a gradient transform is), and **COMP-22** is also what validation says of a fill paint assignment past the end of the hatch stream.
* **COMP-25** is a parent that is not a clip with a smaller index (which also covers a parent past the end of the table), and **COMP-21** is also an item assigned to two clips.
* **COMP-28**, entity or record keys that are not strictly ascending, where a layout says they are (the entities of 24, the hatch records of 25). Equal keys are refused by it too, so a key cannot be assigned twice.
* The overflow of `first + count` is COMP-08 for kind 1 (an entity id at or above the limit) and COMP-03 for the others.

Rules validation checks, given the other streams (the same split as COMP-16 and COMP-17: decoding needs only the payload, validation needs the artifact):

* **COMP-22**, an item that does not exist: an entity that is not in the geometry, a hatch record, raster or text record past the end of its stream, or a stroke that is not drawn (a hidden entity cannot be painted).

An item that no run names is painted before every listed item, in the legacy stage order of the viewer (paper, fills, strokes, text). A file without this stream paints in that order for all of it. So a producer that wants a partial order can list only what must be ordered, and a reader that skips the stream loses nothing it was ever promised.

### 2.3 Capability 24, `stroke_paint`

The line style stream says how thick and how dashed, and deliberately not what colour. This stream says the colour. It has the shape of the line style stream: a table, then pairs.

| part | content |
|---|---|
| header | `u32` colour count (1 to 65,536), `u32` assignment count (at most 2^24) |
| colours | `u32` each, an RGBA word exactly as in the hatch stream (`0xRRGGBBAA`, straight alpha, sRGB) |
| assignments | pairs of an entity (`u32`) and a colour index (`u32`), entities strictly ascending |

The alpha byte is the stroke alpha (`CA` in a PDF graphics state). An entity with no assignment has no colour in the file and the viewer's own style applies, which is today's behaviour, so the stream is a strict addition. A colour index with no entry is COMP-09.

Text takes its colour here too. A text record names an anchor entity (section 6.6), and the colour assigned to that entity is the colour of the text. One mechanism, and no colour field is added to the text stream.

### 2.4 Capability 25, `fill_paint`

The hatch stream says which rings are filled and gives a flat colour. This stream refines a hatch record: which rule, and a gradient or a tiling pattern in place of the flat colour. The flat colour stays in the hatch record on purpose. It is what a reader that does not know 25 draws, so the producer writes there the best single colour for the area (the middle stop of a gradient, the mean colour of a tile), and the fallback degrades to a flat tint, not to a hole.

The payload is three `u32` counts (gradients at most 4,096, tilings at most 1,024, assignments at most 2^20), then the gradients, then the tilings, then the assignments.

**Assignment**, 12 bytes, records strictly ascending:

| offset | size | field |
|---|---|---|
| 0 | 4 | hatch record index |
| 4 | 1 | rule, 0 nonzero, 1 even-odd |
| 5 | 1 | paint, 0 the hatch record's own fill, 1 gradient, 2 tiling |
| 6 | 2 | reserved |
| 8 | 4 | index into the gradients (paint 1) or the tilings (paint 2), zero for paint 0 |

A hatch record with no assignment keeps what the viewer did before this stream existed, even-odd by ring nesting, so a file written before 25 draws as it always did. A producer that writes the stream writes the rule for every record it assigns.

The rule applies to **all** the boundary rings of the record together, which is what a PDF fill does with a compound path: the winding number of the whole ring set decides, so a donut written with two rings wound the same way is a ring under even-odd and a disc under nonzero, and a pentagram has an empty centre under even-odd and a filled one under nonzero.

**Gradient**, variable:

| part | size | content |
|---|---|---|
| kind | 1 | 1 axial, 2 radial |
| extend | 1 | bit 0 paint before the first stop, bit 1 after the last; other bits zero |
| stops | 2 | stop count, 2 to 256 |
| transform | 48 | 6 x f64 `[a b c d tx ty]`, from gradient space to source space, in PVec's `Transform2` order |
| coordinates | 32 or 48 | axial: `x0 y0 x1 y1`. Radial: `x0 y0 r0 x1 y1 r1`. All f64 |
| stops | 12 each | f64 offset, `u32` RGBA word |

Axial: the parameter of a point is its projection on the segment from `(x0, y0)` to `(x1, y1)`, 0 at the start and 1 at the end. Radial: the PDF type 3 rule, the circle family from `(x0, y0, r0)` to `(x1, y1, r1)`, and a point takes the largest parameter whose circle has a non-negative radius and passes through it; a point no such circle reaches, or one outside the extend flags, is left unpainted (transparent). Offsets are finite, in 0 to 1 and non-decreasing, a repeated offset is a hard step. Colours are interpolated as straight sRGB components and then premultiplied, which is what PDFium does with a device colour space. Refused: a zero-length axis, a negative radius, two identical circles of equal radius, a singular transform (COMP-23); stops out of order or outside 0 to 1 (COMP-24).

The producer samples a PDF function (type 0, 2, 3 or 4) into stops and converts the colour space to sRGB before it writes. The PVec never holds a PDF function, which keeps the reader free of a PostScript calculator. The cost is that a smooth function becomes up to 255 straight pieces, and the producer reports the worst error it accepted.

**Tiling**, variable:

| part | size | content |
|---|---|---|
| paint type | 1 | 1 coloured, 2 uncoloured (painted in the colour of the hatch record) |
| flags | 1 | zero |
| primitives | 2 | primitive count, 1 to 256 |
| steps | 16 | f64 `x_step`, f64 `y_step`, both finite and positive |
| cell box | 32 | f64 `x0 y0 x1 y1`, non-empty |
| transform | 48 | 6 x f64, from pattern space to source space |
| primitives | variable | see below |

A primitive is a kind (1 filled polygon, 2 stroked polyline), a rule byte (polygons only), a `u16` point count (3 to 4,096 for a polygon, 2 to 4,096 for a polyline), a `u32` RGBA word (ignored in an uncoloured tiling), an f64 line width (zero for a polygon) and then the points as f64 pairs, in pattern space. A tiling holds at most 8,192 points in all (COMP-27).

This is the one place in the set where a stream holds coordinates itself, and I want to be clear about why that does not break the rule that streams hold no curve. The cell is not in source space, has no `EntityId`, cannot be selected or measured and is not seen by the geometry oracle or the spatial index. It is paint, in the same way as a gradient's stops are, and it is bounded hard. The alternative of making the cell's contents entities in a reserved space was rejected in the ADR.

### 2.5 Capability 26, `clip`

A clip region is a set of rings of entities that already exist, plus a rule, exactly the way a hatch boundary is. Clips nest, and the items they apply to are named by paint item.

| part | content |
|---|---|
| header | `u32` clip count (at most 65,536), `u32` run count (at most 2^20) |
| clips | each: rule (u8, 0 nonzero or 1 even-odd), 3 reserved bytes, `u32` parent (`0xFFFFFFFF` for none), `u32` boundary kind (1, references), `u32` boundary count (1 to 4,096), then that many entity ids |
| runs | 16 bytes each: `u32` kind, `u32` first, `u32` count, `u32` clip index |

The region of a clip is the area its rings enclose under its rule, intersected with the region of its parent. An item the runs assign to a clip is drawn only inside that region; an item no run names is not clipped. The parent of clip k is a clip with a smaller index, so there is no cycle (COMP-25), and a chain is at most 8 deep (COMP-26), which is the number of bits a stencil buffer gives a renderer to intersect with. A boundary kind other than 1 is refused as embedded geometry (COMP-10), as in the hatch stream.

A clip applies to the geometry of the item, and to a raster in addition to its `visible_box`.

The rings are ordinary entities, written hidden by the line style stream, so a clip circle is a closed polyline of the producer's preview flattening, the same one a filled circle gets. The geometry stays whole: nothing is cut, so a snap or a measurement still finds the authored vertex, and the clip is a statement about display.

## 3. What a reader does

* **Unknown streams.** The reader reads the streams it knows and nothing else. A stream it does not know, or knows at another major version, is a `Skipped` report with its reason, never an error (`docs/COMPONENT_STREAMS.md`).
* **Painting.** With 23, the viewer paints the listed items back to front, each inside its clip. Without it, it uses its legacy stage order. Items are never reordered across a change of kind unless their bounds are disjoint (section 5.3).
* **Rasters.** The `band` byte of a raster placement stays and is read by a reader that has no 23. A reader that has 23 and finds a raster named in it takes the position from the order and ignores the band.
* **Absence is not an error.** A hatch record without an assignment, an entity without a colour, an item that is not in the order: all draw as they did before.

## 4. Compatibility

The streams are additions of new capability ids. No byte of an existing stream changes, so every existing reader reads every new file, and every new reader reads every old one.

| reader | new file | what it shows |
|---|---|---|
| geometry only, no component streams | opens degraded, `Uncomponented` | the linework, as today, plus the hidden fill rings it cannot know are hidden |
| v1 component reader (draws 14 to 17, 21) | opens degraded, 23 to 26 reported `NotImplemented` | flat fills at the colour in the hatch record, in the legacy order, with alpha, no clips, default stroke colour, text |
| a reader that implements all of them | opens | the page as the PDF paints it |
| a reader that implements 23 to 26 reading an old file | opens | exactly what the old file drew |

Why new streams and not new versions of the hatch and line style streams: a payload is consumed exactly (COMP-02), so a field cannot be appended to hatch 1.0, and a major bump makes every v1 reader skip the whole stream and draw no fills at all. Four new ids cost nothing and lose nothing.

The compat matrix (`tests/compat/matrix.txt`) gets rows for the reader and file pairs above, with the outcomes written in the fixture, as for capabilities 21 and 22. The registry lint accepts the change because it adds capabilities and raises the format version by a minor.

The reference viewer in this repository (ADR 0020) and every other reader of the container are covered by the same decision table: they open a file with these streams degraded and report them. `maplibre-pvec` is archived and needs nothing. No other repository has to change for compatibility, only the ones that want the new drawing.

## 5. The renderer in PlanViewer

What PlanViewer changes, in the order a frame meets it. Crate names are those of the repository.

### 5.1 Fill rule (`planviewer-pvec` components, `planviewer-render` fill pass)

`append_fill` today nests rings by containment and triangulates each ring with earcut as a simple polygon. That cannot honour a rule: a self-intersecting ring is filled through its centre whatever the rule, and a donut wound the same way gets a hole whatever the rule. The replacement is a fill tessellator that takes a ring set and a rule and handles self-intersection (lyon's `FillTessellator` with `FillRule::EvenOdd` or `NonZero`). The input is the hatch record's rings joined as now; the rule comes from the assignment of 25 and is even-odd when there is none.

Two consequences to check before it lands: the wasm size budget in `docs/phase2.md` (the tessellator is the largest new dependency of the plan), and the output for files without 25 (a pentagram changes, deliberately, because the old result was wrong).

### 5.2 Stroke colour

`StyleOptions` gets `fileColours` (default true). The precedence for a stroke becomes: the host's style hook, then the colour for the line style name, then the colour of the file (24), then the colour for the layer name, then `ink`. The doc line "Colour is not in the file" in `docs/05-viewer-capabilities.md` changes to say the file may carry it. Width needs nothing from the viewer: the line style stream already has it, and the unit rule is in 6.3.

### 5.3 Paint order (`DRAW_ORDER`, `gpu.rs`)

`DRAW_ORDER` is a fixed array of six stages walked once per frame. With 23 the frame is a list of **batches**, each a run of consecutive items of one kind with one clip. The legacy order is the batch list `[paper, raster 0, fills, strokes, text, raster 1]`, so `DRAW_ORDER` stays as the plan a file gets when it has no 23 and nothing else changes for those files.

* The `pvec-scene` crate (`ScenePlan::build`) is that scene builder's first half: it returns the batches and a `Degrade` list (`TooManyBatches` past `Policy::max_batches`, 4096 by default, and `StreamDamaged` for a stream 23 that does not decode or names an item the page lacks), and in both cases the plan is the legacy order. Clips, sampling and tessellation come later or stay in the viewer.
* The scene builder expands the runs into items, groups them into batches, and the render pass loops over batches instead of stages. A raster item draws one placement (the raster pass already works per placement and band; it needs a per-placement entry point).
* A fill batch and a stroke batch share the one render pass, as now. Blending stays premultiplied.
* The cost of an alternating page is one draw call per change of kind, which is fine up to a few thousand. Past 4,096 batches the builder merges adjacent batches whose bounds do not overlap, which does not change the picture because a fill and a stroke that do not touch can be painted in either order; if it is still over, it falls back to the legacy order and counts `paintOrderDegraded` in the scene stats. This is a measured ceiling, not a guess: the PR that lands it carries the frame-budget numbers.
* A translucent stroke of several pieces that overlap itself would double-blend where its segments join. The stroke pass already draws a polyline as one run; the batch work is the place to resolve it (stencil once per entity), and the alpha stroke region is the test.

### 5.4 Clips (a stencil attachment)

The render pass gets a depth-stencil attachment, allocated only for a scene that has a clip stream, so the files that exist pay nothing. The scheme is one bit per level with a write mask: a clip at depth d (the root is depth 1) owns stencil bit d-1, so a chain nests at most 8 deep, the 8 bits of the buffer. The rings are tessellated by the same tessellator as fills, which resolves the rule (non-zero or even-odd) into triangles that do not overlap, so the stencil never has to count a winding number and nothing mixes an increment with an invert on the same bits. For each distinct clip, the batches under it are drawn like this:

1. Write the region. Draw the clip's triangles with write mask = bit d-1, compare mask = the parent's chain mask (bits 0 to d-2), a stencil test of equal to that chain mask, and the replace op with a reference of all ones. A root clip has an empty chain mask and writes everywhere its triangles cover. The result is that bit d-1 is set exactly where the clip is inside its parent's region.
2. Draw the batch with write mask 0, compare mask = the chain mask including bit d-1, and a stencil test of equal to that chain mask.
3. Clear what was written. Draw the bounds of the clip's triangles with write mask = bit d-1 and the replace op with a reference of zero, so the bits of the parents are untouched and the next sibling starts clean.

A separate parity pass is not needed, and a clip that nests deeper than 8 is refused by COMP-26 before a viewer sees it.

### 5.5 Gradients

The fill pipeline gets a variant whose vertices carry a gradient index. The stops of every gradient are resampled to 256 texels, one row per gradient in a small texture (4,096 rows at most), so the fragment shader does one lookup. The fragment maps the source position through the inverse gradient transform, computes the parameter (the projection for axial, the quadratic of the circle family for radial), discards what is outside the extend flags and what no circle reaches, and samples the row. Precision: positions are taken relative to the camera centre like every other pass.

The numbers come out of `pvec-scene::sample_gradient`: a row of `texels + 2` RGBA `f32` texels (planviewer asks for 256), where the first and last texel are guards that are the end colours when the matching extend flag is set and fully transparent when it is not, and the texels between sample the stops (exactly the first and last stop at the ends, a hard step at a repeated offset, straight alpha untouched).

### 5.6 Tiling

A tiling is drawn into an offscreen RGBA texture holding one cell (and the neighbouring copies when the cell box is larger than the step), at a size chosen from the device pixels one step covers, rounded to a power of two between 16 and 1,024 so it is redrawn when the zoom crosses a bucket and not every frame. Mipmaps are generated. The fill fragment maps the source position through the inverse pattern transform, takes the fractional part of position over step, and samples with repeat. An uncoloured tiling uses the texture as a coverage mask times the record's colour. When a step is under two device pixels the shader reads the top mip, which is the mean colour, so a dense pattern fades to a tint and not to noise, as dashes already do.

`pvec-scene::sample_tiling` gives the primitive list for that texture as plain vectors: `ceil(cell width / x_step)` by `ceil(cell height / y_step)` copies of the cell (one when the cell box fits inside the step), neighbours first and the cell itself last, with the copy offsets already added to the points. More than 64 copies is reported as `Degrade::TilingTooLarge` and the area keeps its flat colour.

### 5.7 Text

The viewer already reads the text stream and draws it. What can still be wrong is the metrics: the glyphs come from the browser's face and the PDF's advance widths are Helvetica's. The first PR of the text item is a red test on the title block of `fid-blueprint-mix`, and the width field is added to the text stream (a 2.0 text stream, which a v1 reader skips) only if that test shows the difference is over the threshold. I would rather measure than add a field I might not need.

### 5.8 Negotiation and the descriptor

`viewer_capabilities` and the set of drawn streams in `stream.rs` add 23 to 26. `check_descriptor_capabilities` accepts a `pvec.preview.*` label as it does a `pvec.hot.*` one (it names a producer preview, not a capability), which removes the workaround the fidelity specs carry and the NEGOTIATE-01 the server's descriptor triggers today.

## 6. The conversion

Today the server's `convert_pdf` turns each path into polylines (style 2 for a fill, 1 for a stroke), cuts rectangular clips, drops white fills, and writes no stream. It does this itself, from `PdfPathEntity` of `libviprs`. The design moves the mapping into libviprs (`pvec::pdf_page_to_pvec`), where PlanViewer's test generator (`tests/pdf-compare/gen`) and the server both call it. That is two callers, which is the reuse that justifies a function in the library and not a copy in each, and it means the fidelity specs measure the code the server runs and not a lookalike.

### 6.1 What libviprs has to track

`PdfPathEntity` has the fill colour (libviprs 1545, not touched here) and `stroke_params` with the raw `w` and `d`. Needed, per painted path:

* the stroke colour, from `G RG K`, `CS`/`SCN` in device spaces (gray, RGB, CMYK; the conversion of CMYK is the documented one and its error against PDFium is measured, not assumed);
* the fill and stroke alpha from `ca` and `CA` in the `gs` graphics state, folded into the alpha byte;
* the CTM at paint time, so a width can be taken to source units (section 6.3);
* `sh` as a record (the shading dictionary resolved, its matrix, the clip in force), and a pattern colour as a record (tiling cell contents, matrix, paint type, or the shading pattern);
* each text-showing operator as a record (section 6.6).

### 6.2 Path to entities and streams

| PDF | PVec |
|---|---|
| fill (`f F f*`) | one hatch record, its boundary the closed rings of the path as hidden entities (line style "fill only", visible bit clear), the rule in 25 |
| stroke (`S s`) | one visible entity, the colour in 24, width and dash in 15 |
| fill and stroke (`B b B* b*`) | a hatch record and a visible entity, and the closed ring is the same entity, which the hatch names as its boundary |
| a compound path | one hatch record whose boundary has every subpath, not one record per subpath: a hole is a ring of the same record |
| `sh` | a hatch record whose boundary is the clip region in force (or the shading's `/BBox`, or the page), 25 gradient |
| pattern fill, tiling | a hatch record, 25 tiling; uncoloured takes the colour in the record |
| pattern fill, shading pattern | as `sh`, with the pattern matrix as the transform |
| image | a raster placement of capability 21, baked by `bake_page_images` (section 6.8) |
| text | a text record and its anchor (section 6.6) |
| white fill, nothing under it | still dropped, as now, **unless** it is painted over something that is drawn (paint order makes this decidable, section 6.5) |

Entity ids are dense, in paint order. The records of the hatch, text and raster streams are in paint order too, so the runs of 23 are mostly one per change of kind.

### 6.3 Widths

The line style stream's explicit width is in source units (inches in the server's files), so the producer writes the PDF line width multiplied by the scale of the CTM at paint time. For a CTM that is not a similarity the scale is the square root of the absolute determinant, and the producer reports the paths where the singular values differ by more than a factor of 1.1, which the format cannot express. A width of 0 is `Thinnest`. The viewer draws a width under one device pixel at one device pixel, which is also what PDFium does.

### 6.4 Clips and closed-ring cutting

* A clip that is an axis-aligned rectangle is applied to the geometry, as today. A fill is cut as a **closed ring** (a polygon clip, never a polyline clip: a polyline clip can split a ring into open pieces that triangulate wrongly), and a stroke is cut as a polyline. A convex clip such as a rectangle cuts a ring correctly under both rules, so a cut ring keeps its rule.
* Any other clip is not applied to the geometry. Its path is written as hidden closed entities and a clip record of 26, and the items under it are assigned to it. The clips in force are the intersection of the `active_clips` of the entity, so the chain is the nesting (a rectangle in the chain is cut and left out of the record; the others become the record's chain).
* An empty clip drops its items, as the clip policy does today.
* A chain deeper than 8 non-rectangular clips draws the items under the inner ones with the first 8 and reports the rest.

The old readers see the geometry unclipped, which is what they see now.

### 6.5 Order and the white-fill rule

Order comes from `provenance.sequence`. With it, the producer writes 23, and the white-fill rule can be right where it is now only a guess: a white fill matters if something earlier is drawn under it. The producer keeps a white fill when an earlier item's bounds overlap it, and drops it otherwise, which keeps the common blank-page case small and the covered-grid case correct.

### 6.6 Text

The text stream has records of an entity, an anchor, a height, a rotation and a string, and no font. The producer writes, per text-showing operator in render mode 0 (fill):

* the string, decoded through the font's encoding or `/ToUnicode` (a Type0 font without `/ToUnicode` stays omitted and counted, as now);
* the anchor, the origin of the text rendering matrix in source units, and the rotation, from the matrix;
* the height as the **cap height** (the stream's meaning, `docs/05-viewer-capabilities.md`), which is the font size times the CTM scale times `/CapHeight` over 1000 from the font descriptor, 0.7 when it has none;
* an anchor entity: the baseline of the run as a hidden two point polyline whose length is the advance from `/Widths`. It gives the text bounds for the spatial index and a thing to hang the colour of 24 on;
* nothing for render mode 3 (invisible), a count for 4 to 7 (clipping, as now).

### 6.7 Descriptor

The server stops writing `pvec.preview.pdf-cubic-polyline` among the required capabilities. It names the geometry capabilities the file really uses, and the component and raster capabilities go through the manifest, where they are optional. The viewer accepts the label as well (5.8), so a server older than the viewer, and the reverse, both work.

### 6.8 Images

The raster side already exists in the format (ADR 0018) and in libviprs (`bake_page_images`, issue 1332). The change is to use it: the server bakes the images of the page into capability 21 placements, with their soft masks as alpha and their clips as `visible_box` or baked alpha, and names them in 23 so they are painted where the PDF paints them. The pyramid of the whole page that the descriptor points at stays what it is, a first-useful preview under the vectors while they stream, and it is no longer needed to carry the pictures. A page whose images are over `max_images` or `max_image_bytes` keeps today's behaviour (the pyramid carries them) and the report says so. This reverses one line of the planviewer 32 list ("by design, the pyramid") for the 12 regions that need it, and the reason is in the ADR.

## 7. The 45 regions

The ids are the `fid-<fixture>/<region>` keys of `notRepresentable` in `tests/pdf-compare/pdf-compare-fidelity.spec.ts`, mode `vector-hatch`, at `fit`. Letters are the PRs of chapter 8.

| # | regions | cause | enabled by |
|---|---|---|---|
| 2 | fid-fills/star-evenodd-centre, fid-fills/donut-nonzero-inner | no fill rule, ring-nesting heuristic | **P2** (rule in 25), **V1** (tessellator), **S1** |
| 1 | fid-fills/stroke-frame | stroke colour and width not carried | **P1** (24), **L1**, **V2**, **S1** |
| 4 | fid-clips/circle-clip-outside, fid-clips/evenodd-clip-hole, fid-clips/clip-outside-triangle, fid-clips/half-circle-above | clip is not a rectangle | **P3** (26), **L5**, **V4** (stencil), **S2** |
| 5 | fid-clips/half-circle-below, fid-alpha/alpha-grey-plain, fid-blueprint-mix/bp-building-fill, fid-blueprint-mix/bp-building-b, fid-blueprint-mix/bp-parcel-tan | a stroke the PDF paints under or clips away from an opaque fill is drawn over it | **P1** (23), **V3** (batches), **L4**, **S2** (half-circle-below also needs the clip, **P3**/**V4**) |
| 12 | fid-shading-axial/axial-2stop-ramp, axial-3stop-ramp, axial-angled-star, axial-extend-left, axial-extend-right; fid-shading-radial/radial-concentric, radial-cone, radial-inner-extend, radial-corner-extend; fid-shading-bounded/hexagon-shade, band-shade; fid-blueprint-mix/bp-legend-ramp | `sh` is not extracted and a fill is a flat colour | **P2** (gradient), **L2**, **V5**, **S2** (band-shade also the compound path of 6.2, axial-angled-star the rule) |
| 3 | fid-tiling/tiling-checker, tiling-hatch, tiling-triangle | pattern fills are not extracted | **P2** (tiling), **L3**, **V6**, **S2** |
| 5 | fid-alpha/alpha-red-only, alpha-overlap, alpha-blue-only, alpha-over-grey, alpha-stroke-over-fill | `ca` and `CA` are not tracked | **L1** (alpha), **S1**; alpha-stroke-over-fill also **P1** and **V2**; alpha-over-grey and alpha-overlap also **V3** |
| 12 | fid-raster-mix/raster-quadrant-red, raster-quadrant-green, raster-quadrant-blue, raster-ramp, raster-over-fill, white-over-raster; fid-raster-smask/smask-ramp-left, smask-ramp-right, smask-ramp-all, smask-opaque; fid-blueprint-mix/bp-scan-paper, bp-overlay | images and soft masks are only in the pyramid | **L6** (bake into 21), **V3** (rasters as items), **S3**; raster-over-fill, white-over-raster and bp-overlay also **L1** |
| 1 | fid-blueprint-mix/bp-text | no text record | **L7**, **S2**; **V7** only if the metrics test is red |

2 + 1 + 4 + 5 + 12 + 3 + 5 + 12 + 1 = 45.

Two places where I do not know yet, and say so: `half-circle-below` is listed in the issue under paint order but its hairline is also "clipped away", so it may need both 23 and 26; and the bp-text threshold may or may not need the width field. Each is settled by the red test of the PR, not by this page.

Acceptance for each row is the same: the test names are the existing specs, `test.fixme` is removed from the entry in the PR that makes it pass, `PV_FIDELITY_ALL=1` is the check that nothing listed still passes by accident, and the numbers are the thresholds already in `thresholds.ts` (4/255 flat, 8 ramps, 10 rasters, 12 patterns after the blur). No threshold moves.

## 8. The PR plan

Sizes are lines of change including tests: S under 300, M 300 to 800, L 800 to 1,500. Every PR starts from red tests, and says so.

**Format (libviprs/pvec)**

| PR | content | size |
|---|---|---|
| P0 | this design: ADR 0021 and this page | docs |
| P1 | registry entries and codec for 23 and 24, rules COMP-21, COMP-22, compat matrix rows, format 1.3.0 | M |
| P2 | 25: rule, gradient, tiling, COMP-23, 24, 27, limits in `SECURITY_LIMITS.md`, fuzz target | L |
| P3 | 26: clips, COMP-25, 26 | M |

**Producer (libviprs/libviprs)**

| PR | content | size |
|---|---|---|
| L1 | stroke colour, `ca`/`CA`, CTM scale on `PdfPathEntity` | M |
| L2 | `sh` and shading patterns, function sampling to stops | L |
| L3 | tiling patterns: cell contents, steps, paint type | L |
| L4 | `pdf_page_to_pvec`: hatch (17), hidden line style, 23, 24, the white-fill rule | L |
| L5 | clips: rings, chains, 26 | M |
| L6 | images: `bake_page_images` into 21 and named in 23 | M |
| L7 | text records and anchors | M |

**Server (iasbuilt/server) and its tests (iasbuilt/server-tests)**

| PR | content | size |
|---|---|---|
| S1 | `convert_pdf` calls `pdf_page_to_pvec`, descriptor labels (6.7), hatch, line style, 24 | M |
| S2 | the rest of the streams once libviprs writes them (25, 26, text) | S |
| S3 | the image bake and its limits | S |
| T1 to T3 | a companion spec for each, decoding the registered artifact | S each |

**Viewer (planviewer, tracker #32)**

| PR | content | size |
|---|---|---|
| V0 | the pin bump, negotiation (5.8), stats | S |
| V1 | tessellator and fill rule (5.1) | M |
| V2 | stroke colour (5.2) | S |
| V3 | paint order batches, rasters as items (5.3) | L |
| V4 | clips (5.4) | L |
| V5 | gradients (5.5) | L |
| V6 | tiling (5.6) | L |
| V7 | text metrics, only if red (5.7) | S |
| V8 | the generator calls the libviprs mapper, `vector-hatch` mode retires into `vector-only`, `notRepresentable` empties | M |

Order: P0, then P1 to P3 in parallel. V0 follows P1. V1 to V6 each need only their stream's codec, because `gen/` writes the streams itself from the paths as it writes the hatch variant now, so the viewer work does not wait for libviprs and the server. L1 can start at once, L2 to L7 as their stream lands in the codec. S1 needs L4, S2 needs L2, L3, L5 and L7, S3 needs L6. V8 goes last: it replaces the hand-built streams with the real conversion, and that is the end to end proof.

The order inside the viewer is by risk, not by region count: V1 and V2 are small and unflip three regions, V3 is the one that every later PR leans on for correct layering, V4 reuses V1's tessellator, and V5 and V6 share the fill pipeline variant.

## 9. Tracking

* planviewer: issue 32, which already holds the checklist. Its list is extended with the letters above.
* libviprs/pvec: one issue for P1 to P3.
* libviprs/libviprs: one issue for L1 to L7.
* iasbuilt/server: one issue for S1 to S3.
* iasbuilt/server-tests: one issue for T1 to T3.

Each issue holds its PR list as a checklist in the body and nothing else is opened for leftovers.

## 10. What this does not do

* Blend modes, transparency groups, knockout and overprint. `ca` and `CA` are constants; a soft mask in the graphics state (not on an image) stays reported as unsupported, as in the clip policy.
* Line caps, joins, the miter limit. They are not in any of the 45 regions. They would be fields of a line style 3.0, and the stream has a version for that.
* Text clipping render modes, and fonts. The stream names no font.
* Mesh shadings (types 4 to 7) and function-based shadings (type 1). Counted and reported as today.
* Exact curves for fills. The rings are the preview flattening the server already writes; exact Bezier entities are a separate decision.
* Anything that makes a stream required. Every one of the four is optional for good.

## 11. Risks

| risk | what I do about it |
|---|---|
| the tessellator is heavy for the wasm budget | measure in V1 against `docs/phase2.md`; the budget is raised in the same PR if it has to be, with the number |
| stencil support differs on a downlevel (WebGL2) path | the stencil attachment is the same on WebGPU and WebGL2 backends of wgpu; V4 runs the browser grid |
| a page with tens of thousands of alternating items | the ceiling and merge in 5.3, measured in V3 |
| a smooth PDF function becomes up to 255 straight pieces | the producer reports the worst error, and the threshold for ramps (8/255) is the judge |
| CMYK differs between libviprs and PDFium | measured with a CMYK fixture in L1; the fidelity fixtures are RGB, so no region depends on it |
| text advance widths differ | V7 exists for it |
