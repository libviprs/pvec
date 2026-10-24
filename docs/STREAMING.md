# Viewport streaming

How a reader decides which bytes of a `.pvec` file a viewport needs, and reads only those. Chapter 1 is the spatial directory (issue P5.1), chapter 2 payload ownership and duplicated references (issue P5.2). Later chapters (straddlers, tiers, read planning) build on it. The code is the `pvec-stream` crate; the container underneath is `docs/CONTAINER.md`.

## 1. The spatial directory

Viewport I/O is decided by the directory, and the directory is built over source-space bounds: `f64` boxes in the unit the manifest declares, with no tile grid, no display zoom and no web-mercator level anywhere in it. So one file serves any display scale and any projection a consumer picks later, and the directory block is byte-identical whatever display scale the producer wrote into its metadata, because there is nothing in its input to carry one.

The index is the one ADR 0005 selected from P1.6's simulation (`docs/spikes/01-6-spatial-index.md`): a packed R-tree over a Hilbert ordering of entity centres, every payload owned by exactly one cell, small cells. Of the loose quadtree (M1), the R-tree (M2), duplicated references (M3) and leaf cells with long-entity buckets (M4), M2 read the fewest bytes in the interactive steps on `million` (29.08 MB at 64 / 32, against 46.45 MB for the quadtree) and on `stress-long` (15.89 MB against 54.80 MB). A long entity widens its cell's bounds, which is what P5.3 (straddlers) answers; this chapter does not.

### 1.1 Layout

The directory is one block of kind 5 (`spatial`, see CONTAINER.md section 4.2) with the codec 0, so a reader fetches it in one range of the length the block directory gives. The block is optional (must-understand clear): a reader that does not know it skips it, and a file without one is valid. The block directory entry carries the node count as its count and the box of the root as its bounds, and its `cell` is 0. The manifest's directory-pointer kind 3 stays reserved and unused: the block directory already locates the block.

All integers are little-endian, all floats IEEE 754 binary64, all reserved fields zero. The header is 64 bytes:

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | magic | bytes | `PVS1` |
| 4 | 2 | version | u16 | 1 |
| 6 | 2 | flags | u16 | zero |
| 8 | 4 | node_count | u32 | nodes in the block |
| 12 | 4 | leaf_count | u32 | cells, which are the first `leaf_count` nodes |
| 16 | 4 | height | u32 | levels, 1 for a single cell, at most 8 |
| 20 | 4 | fanout | u32 | most children of an internal node, 1 to 256 |
| 24 | 4 | leaf_capacity | u32 | most entities the writer put in a cell, at least 1 (informational) |
| 28 | 4 | entity_count | u32 | entities in the cells |
| 32 | 32 | reserved | bytes | zero |

Then `node_count` nodes of 48 bytes, level by level: all leaves, then level 1, up to the root, which is last.

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 8 | min_x | f64 | source-space bounds of everything under the node |
| 8 | 8 | min_y | f64 | see `min_x` |
| 16 | 8 | max_x | f64 | see `min_x` |
| 24 | 8 | max_y | f64 | see `min_x` |
| 32 | 4 | first | u32 | leaf: its cell id, from 1 and strictly ascending. Internal: index of its first child |
| 36 | 4 | count | u32 | leaf: 0. Internal: number of children, which are `first..first+count` |
| 40 | 4 | entities | u32 | entities under the node |
| 44 | 2 | level | u16 | 0 for a leaf, one more than its children otherwise |
| 46 | 2 | reserved | u16 | zero |

A cell's entities are in the hot blocks whose `cell` field is the leaf's id. The block directory sorts blocks by kind, then cell, then geometry type, so the blocks of one cell are adjacent and a cell names them as a range (`Cell::blocks`).

**Cell 0 is reserved.** In the block directory, 0 is "no cell": the advanced curve set, the component set and the spatial directory itself are in it. The directory therefore never uses 0 for a leaf, and a file that carries a directory is refused when a hot block is in cell 0 (SPD-10), because the entities in it would be invisible to every query. The block kind says what a block in cell 0 is, as it did before. The advanced curve set is one set the container cannot split by cell, so no leaf holds it; `SpatialDirectory::advanced_bounds` reports its bounds beside the cells, and a cell references its curves by the row of the record block instead of holding them (section 2.8).

Per-entity references with their own bounds inside a cell are not in this version: with every payload owned by one cell (ADR 0005) there is nothing to duplicate, and a per-entity list would not fit the 2 MiB open budget at a million entities. See the scope note on issue #10.

The constants are named in `crates/pvec-stream/src/directory.rs`, with P1.6's measurements beside them: `CELL_CAPACITY` 64, `FANOUT` 32, the split policy `HilbertPack` (sort by the Hilbert key of the bounds centre on a 65,536 grid, ties by entity id, cut every `CELL_CAPACITY`), `MAX_FANOUT` 256 and `MAX_HEIGHT` 8 as the ceilings a file may declare. Tuning them against real drawings is P8.6.

### 1.2 Invariants

* Every node's bounds contain the bounds of each child. A leaf's bounds contain the bounds of each of its hot blocks.
* Children tile the level below exactly and in order: node `i` of level `l` starts at the child after the last child of node `i-1`. Every node has one parent, the root is alone on the top level, and a child is always stored before its parent, so a pointer can only point backwards.
* A node's entity count is the sum of its children's; a leaf's is the total of its blocks' counts; the root's is `entity_count`.
* Cell ids ascend strictly through the leaves, from 1, and every leaf has at least one block.
* The directory is built from boxes in source space alone and from nothing about their order of arrival: the same set gives the same bytes.

### 1.3 What a reader validates before it traverses

`SpatialDirectory::parse` checks all of the following before a query runs, and `open` reads the block through the container reader, so the checksum, the block ceiling and the allocation accountant of P4.8 apply first. The length is held to the header before any node is allocated, so a header that claims four billion nodes costs nothing. The walk itself is iterative with a stack of at most `MAX_HEIGHT * MAX_FANOUT` entries, and the tree cannot contain a cycle because a pointer is accepted only if it points strictly before its parent.

| rule | meaning |
|---|---|
| SPD-01 | the block is shorter than its header, or its length is not the header plus 48 bytes per declared node |
| SPD-02 | a magic, version, flags or reserved field that is not defined, a fanout of 0 or above 256, a leaf capacity of 0, or a height of 0 or above 8 |
| SPD-03 | more entities declared than the artifact ceiling of the reader's limits (P4.8) |
| SPD-04 | node bounds that are not finite, in range and ordered |
| SPD-05 | a child pointer outside the nodes, or one that does not point strictly before its parent (a cycle) |
| SPD-06 | a node that breaks the shape: a leaf with children, an internal node with none or more than the declared fanout, children that are not one level down or do not tile the level below, a root that is not alone on the top level, or a leaf count that differs from the header |
| SPD-07 | an internal node whose bounds do not contain the bounds of a child |
| SPD-08 | an entity count that is wrong: a leaf with none, an internal node that is not the sum of its children, or a root that is not the header's total |
| SPD-09 | a leaf cell id that is 0 or does not ascend strictly from one leaf to the next |
| SPD-10 | the cells and the hot blocks disagree: a hot block in cell 0 or in no leaf, a hot block without bounds or with bounds outside its leaf, a leaf with no block, blocks of one cell that are not contiguous, or a leaf whose entity count is not its blocks' total |
| SPD-11 | more than one spatial directory block, or a block entry whose count is not the node count |
| SPD-12 | a directory cannot be built from the input: no entity, a duplicate entity id, or more entities than NP-18 allows |
| SPD-13 | the container refused the read (the error carries its own rule) |
| SPD-14 | a box that is not finite, in range and ordered |

### 1.4 Query order

`SpatialDirectory::query(&Aabb)` yields the cells whose bounds touch the box (closed boxes: touching at an edge or a corner counts), in ascending cell id. Cell ids ascend through the leaves and the leaves are in Hilbert order of their entities' centres, so this is the Hilbert order of the drawing, and a reader that fetches cells in the order they come gets spatially adjacent cells together. A cell is yielded at most once. Entities that tie on every key (identical bounds) are assigned to cells in ascending entity id, so ties are ordered too: entity `i` of 640 identical boxes is in cell `i / 64 + 1`, and the order does not depend on the order the entities arrive in. The tests assert both on a fixture of identical boxes.

The query is exact for the cells: no entity whose bounds intersect the box is in a cell the query skipped, because a node's bounds contain everything under it. It is conservative inside a cell: a cell the query yields may hold entities that do not intersect the box (visibility here is bounds overlap, as ADR 0005 says, and a long diagonal is fetched without crossing the viewport). The tests check this against a brute-force scan on 62,000 entities with four full-width straddlers.

### 1.5 Measurements

Native x64 on MARS, `rustc 1.99.1`, the test fixture `rooms-1m`: 50 x 50 rooms of 400 plan-like entities over 100,000 x 100,000 (seed 1, one in twenty a wall as long as a room), all lines, written through the container writer with the directory and the default sealing. The shape follows P1.6's `million`; it is generated by the test, because the spike's fixture generator is in a separate workspace.

| what | value |
|---|---|
| entities, cells, nodes, height | 1,000,000, 15,625, 16,131, 3 |
| range reads from a cold start to an open directory | 3: the first 4 KiB (header and manifest), the checksum table and block directory in one read of 1,125,072 bytes, the spatial block |
| the spatial directory block | 774,352 bytes (HEADER_LEN + 16,131 x NODE_LEN), 0.37 of the 2 MiB budget, one range read |
| everything fetched to open | 1,903,520 bytes, which is also under 2 MiB, but only because this fixture is all lines: one hot block per cell, so a block directory of 15,625 entries. A drawing with four geometry types per cell has up to four times that many entries, and that table is the container's, not the directory's |
| wall time of the whole test (write, seal, open, 1,000,000 entities), release | under half a second for the sixteen tests together |

The acceptance numbers are asserted by `open_reads_a_million_entity_directory_in_at_most_four_range_reads_and_under_2_mib`. For comparison the spike's byte model priced the 64 / 32 setting at 516,256 bytes of index at 32 bytes per entry; a node here is 48 bytes because it also carries the child pointer, the entity count and the level, which is what lets a reader validate the tree without trusting it.

### 1.6 Out of scope here

Straddlers and the long-entity bucket ADR 0005 keeps as a safety valve (P5.3), tiering (P5.4), read planning and prioritisation (P5.5, P5.7), and tuning the constants (P8.6).

## 2. Payload ownership and duplicated references

Issue P5.2. An entity that spans several cells is stored once. Its geometry payload is one row of one hot block, the block of the cell that owns it (chapter 1), and no byte of it is copied anywhere else. What a cell may duplicate is a reference to that row: a `PayloadRef` of 8 bytes, the block id and the row, both `u32`. Forcing a long entity up to a common ancestor would congest the root, copying its geometry into every cell would store it many times, and the panel's finding 4 allows exactly the middle path of bounded duplication of lightweight references.

### 2.1 The ownership invariant

* Every entity has exactly one payload slot, so the number of slots in the hot blocks equals the entity count, and a byte-level scan of the blocks finds no control-point run twice. The tests assert both on a fixture with 24 long polylines.
* A reference is at most 8 bytes (`REF_BUDGET`). `PayloadRef` is held to it by a `const` assertion in `ownership.rs`, so a field that grows it fails the build, not a test run.
* A payload has one owner: the placement pass refuses an input that gives one entity twice or one payload to two entities (OWN-05).

### 2.2 The duplication policy

The writer-side placement pass, `place`, takes the opened directory and, per entity, its bounds and its `PayloadRef`. The block ids exist only once the container is laid out (the block directory is sorted by kind, cell and geometry type), so the pass runs on the finished artifact: `scan_payloads` reads the hot blocks and gives every slot its reference. Each entity's reference goes into every cell whose bounds intersect the entity's bounds (closed boxes, the owning cell included), found by one directory query per entity.

`DuplicationPolicy` is the ceiling: the most cells one entity may be referenced from, at least 1 (OWN-07). The default is `DEFAULT_MAX_CELLS`, 16. An entity that intersects more cells than the ceiling is not duplicated further and gets no reference at all: it comes back in `Placement::straddlers` for the P5.3 path (chapter 3). The query stops as soon as the ceiling is passed, so a long entity costs the ceiling and not the number of cells it crosses. A straddler loses nothing in the meantime, because its payload is still in its owner's block and the owning cell's bounds contain it.

### 2.3 The reference arrays

`CellRefs` holds, per cell that has any, its references in ascending order. It has its own byte layout (`encode` and `parse`), checked like any untrusted input: the length is held to the header before anything is allocated. All integers are little endian. The arrays are stored as one container block of kind `refs` (section 2.3.1).

| offset | size | name | meaning |
|---|---|---|---|
| 0 | 4 | magic | `PVR1` |
| 4 | 2 | version | 1 |
| 6 | 2 | flags | zero |
| 8 | 4 | cell_count | cells that have references |
| 12 | 4 | ref_count | references in all |
| 16 | 4 x cell_count | cell ids | strictly ascending, none 0 |
| after | 4 x cell_count | ends | for each cell, the end of its references in the list below; each cell has at least one, and the last end is `ref_count` |
| after | 8 x ref_count | references | `block_id` u32, `slot` u32; strictly ascending inside a cell |

#### 2.3.1 The reference block

The arrays are one block of kind 6 (`refs`, `BlockKind::CellRefs`) holding exactly the bytes above. Its directory entry says it is in no cell (cell 0), its count is the number of cells that have references (`cell_count`), it has no bounds, it is stored as it is, and it is not must-understand: a reader that does not know the kind skips it, as it skips the spatial directory. `ContainerWriter::set_cell_refs(cells, bytes)` sets it. A block kind sorts after every lower kind, so the block lands after the spatial directory and moves no hot block: a writer places in two passes, first the file without the block, then `scan_payloads` and `place`, then the same file with the block, and the block ids the references name are the same in both.

It is read lazily. Neither `ContainerReader::open` nor `SpatialDirectory::open` fetches it (a test asserts that no byte of it is requested); `CellRefs::open(&reader)` fetches it when a caller wants references, and returns `None` for a file without one. `CellRefs::find` looks at the block directory only. The block is one range read in whole, not one per cell. I considered one block per cell (the cell field of the entry is there for it) and dropped it: every block is 64 bytes of the directory a reader fetches at open, and per-cell blocks would add one entry per leaf to the part of the file that has to stay small, to save reading arrays that are 8 bytes a reference. If a drawing's arrays ever outgrow a single read, the layout above already has the cell table in front, so a ranged read of one cell is a change to the reader and not to the format. #117 asks for the lazy read; it is the part of it this answers, the per-entity box is still open there.

Two more rules cover the block: OWN-10 (more than one) and OWN-11 (the entry disagrees with the bytes).

### 2.4 The duplication report and what it measured

`DuplicationReport` is the accounting per artifact: references per entity (mean, p99 by nearest rank, max), the entities that got references and the straddlers that did not, total reference bytes, and reference bytes as a fraction of payload bytes, where payload bytes are the decoded length of the hot blocks (`payload_bytes`).

The fixtures of `tests/issue_11_payload_ownership.rs`, default ceiling 16, in two pipelines. Native x64 on MARS, `rustc 1.99.1`, debug build.

**Before P5.3** the directory is built over every entity, so a long entity widens its cell. This is `tests/issue_11_payload_ownership.rs`, and it still holds a fraction of **2.0** on all four fixtures as the worst case of the placement pass alone:

| fixture | entities | straddlers | references | mean | p99 | max | reference bytes | payload bytes | fraction |
|---|---|---|---|---|---|---|---|---|---|
| rooms (8 x 8 rooms of 400 lines) | 25,600 | 0 | 97,203 | 3.80 | 6 | 8 | 777,624 | 1,036,800 | 0.750 |
| stress-long (rooms 6 x 6 plus 24 long polylines) | 10,255 | 569 | 90,460 | 8.82 | 16 | 16 | 723,680 | 440,200 | 1.644 |
| identical (640 equal boxes) | 640 | 0 | 6,400 | 10.00 | 10 | 10 | 51,200 | 25,920 | 1.975 |
| nine-cells (575 lines and one long polyline) | 576 | 0 | 1,186 | 2.06 | 3 | 9 | 9,488 | 23,416 | 0.405 |

**After P5.3** `classify` runs first, the directory is built over the local entities only, and only they get references; a straddler is in a list (chapter 3) and has none. Payload bytes are the hot blocks of the file plus the decoded bytes of the straddlers (a straddler's payload cannot sit in a leaf of a directory that was built without it, SPD-10, so the test writes the straddlers as a second file with no directory and adds its hot bytes). The numbers come from `tests/issue_131_reference_bound.rs`, which also checks this table against the document:

| fixture | entities | straddlers | references | mean | p99 | max | fraction |
|---|---|---|---|---|---|---|---|
| rooms | 25600 | 0 | 97203 | 3.80 | 6 | 8 | 0.750 |
| stress-long | 10800 | 24 | 34079 | 3.16 | 6 | 9 | 0.620 |
| identical | 640 | 0 | 6400 | 10.00 | 10 | 10 | 1.975 |
| nine-cells | 576 | 0 | 1186 | 2.06 | 3 | 9 | 0.405 |

What changed: `stress-long` fell from 1.644 to 0.620, because the 24 long polylines no longer widen the cells they used to sit in, so no entity reaches more than 9 cells (it was 16, the ceiling, with 569 entities over it). `rooms` and `nine-cells` did not move: `classify` calls nothing long there (zero straddlers, including the one long polyline of `nine-cells`), so the overlap in `rooms` comes from the cells themselves, small by entity count and large in extent, and a short line reaches 3.8 of them on average. A line is also the smallest payload (40 bytes), so an 8 byte reference is a fifth of it. `identical` did not move either, and cannot: 640 equal boxes tie in every cell, so each reaches the ten cells of its tie whatever is classified. That fixture is degenerate, and I state its bound apart.

The stated bounds are a fraction of **0.8** of the payload bytes on the plan-like fixtures (`rooms`, `stress-long`, `nine-cells`) and **2.0** on the degenerate one (`identical`). The first is down from 2.0, and the test asserts both. 0.8 is the measured 0.750 with a little room for a different seed and not a design target.

How the ceiling trades references for fraction on the same pipeline:

| ceiling | rooms | stress-long | identical | nine-cells |
|---|---|---|---|---|
| 1 | 0.002 | 0.006 | 0 | 0.018 |
| 2 | 0.043 | 0.102 | 0 | 0.321 |
| 4 | 0.508 | 0.517 | 0 | 0.402 |
| 8 | 0.750 | 0.620 | 0 | 0.402 |
| 16 | 0.750 | 0.620 | 1.975 | 0.405 |

(`identical` is 0 below 10 because every entity then reaches more cells than the ceiling and is a straddler of the placement pass, with no reference.)

One thing follows and stays as it was. In the P5.1 tree an owning cell's bounds already contain its entity, so any viewport that sees an entity touches its owner: the references are redundant for visibility today, and they earn their bytes only for a reader that wants a cell's whole contents without walking the owners of its long entities, and for the advanced curves (section 2.8), whose payload no cell owns.

### 2.5 Read-side deduplication

`gather` takes the cells a viewport touches (`SpatialDirectory::query`), concatenates their reference arrays, and sorts and deduplicates them: an entity that spans nine touched cells is listed nine times and comes out once (`ViewportRefs::raw` keeps the count before, for the metrics). The result is in `(block, row)` order, so the rows of one block are together. `read_viewport` fetches the blocks through the reader's plan (blocks close together arrive in one request), parses each once, and calls the visitor once per payload with the `EntityId`, the block view and the row, so a payload is decoded once per viewport however many cells referenced it. The tests count it three ways: the visitor on a viewport over nine cells that all reference one long entity, the requests a counting source saw for that entity's block, and a random sweep of 40 viewports that checks every visible entity is yielded exactly once. Taking the deduplication out of `gather` turns the nine-cell test red (it sees the long entity nine times).

### 2.6 What a reader validates

`CellRefs::parse` and `read_viewport` refuse by rule, and the rules are in `OWNERSHIP_RULES`:

| rule | meaning |
|---|---|
| OWN-01 | a reference block shorter than its header, or whose length is not the header plus 8 bytes per declared cell and per declared reference |
| OWN-02 | a magic, version or flags field that is not defined |
| OWN-03 | a cell table that is wrong: a cell id of 0 or one that does not ascend strictly, an empty cell, or ends that do not reach the declared reference count |
| OWN-04 | references inside a cell that do not ascend strictly |
| OWN-05 | a placement input that gives one entity id twice, or one payload to two entities |
| OWN-06 | an entity whose bounds intersect no cell of the directory |
| OWN-07 | a duplication ceiling of 0, or a placement that would emit more references than a u32 can count |
| OWN-08 | a reference that names no payload: a block that does not exist or is not a hot block, or a row past the end of the block |
| OWN-09 | the container refused the read, or a block is not a valid hot block |
| OWN-10 | more than one reference block in the file |
| OWN-11 | a reference block whose entry disagrees with its bytes: it is in a cell, or its count is not the number of cells the arrays hold |

### 2.7 Scope note

Delivered in P5.2: the reference type and its size budget, the placement pass with the ceiling and the straddler hand-off, the report, the reference arrays with their byte layout, and read-side deduplication. Delivered in #131: the container block that stores the arrays with a lazy read (section 2.3.1), the measurements taken again after P5.3 with the lower bound (section 2.4), and the decision on the advanced set (section 2.8).

Not done, and not part of either: where a straddler's own payload lives in a file that carries a directory (the directory refuses a hot block that no leaf owns, SPD-10, so chapter 3's lists point at payloads the writer has no cell to put in), and reading one cell's references without the rest of the block.

### 2.8 The advanced curve set

The advanced curves are one set of six blocks in cell 0, and I keep it one set. A composite names other records of its own block by id, so a curve cannot be moved to another cell without its segments, and a set per cell would need the segments copied or a rule that a composite and everything it names share a cell. The first stores a payload twice, the second is a clustering problem on the writer that the format does not need.

What a cell needs is to know which curves reach it, and that is what a reference is for. An advanced curve is a payload with an owner (the record block) and a row (its position in the records, ascending by entity id), so it is a `PayloadRef` like any other: `block_id` is the record block and `slot` is the row. `scan_advanced` lists them. The placement pass treats them as it treats a hot entity: the curve's bounds go in, a reference goes into every leaf they intersect. `read_viewport` checks a reference to the record block (OWN-08 if the row is past the end) and leaves it alone; `advanced_rows` returns the rows a viewport names, and when that list is not empty the reader fetches the set once through `ContainerReader::advanced` and picks those rows. That replaces the box test of `advanced_in_view` with the cells' own references, and a composite spanning two regions is referenced from both, so a viewport over one end finds it without finding the segment it never touches.

`SpatialDirectory::advanced_bounds` and `ReadPlan::advanced_in_view` stay, for a file that has no reference block: they say the same thing more coarsely, from one box.

## 3. Long entities and straddlers

One 400-metre grid line or a drawing-wide hatch boundary touches nearly every cell, and both obvious answers turn the first read into the whole file. Promoting it to the lowest node that contains it puts it in the root, which every viewport reads. Duplicating a reference to it into every cell it crosses puts it everywhere. There is a third cost I care about as much: a long entity left inside a cell stretches that cell's bounds to cover it, and a stretched cell is fetched by every viewport that touches the stretched box, whether or not anything in the cell is on screen. So I keep long entities out of the cells.

### 3.1 The design

An entity is a *straddler at a level* when its longer side exceeds that level's cell size by `STRADDLER_FACTOR`, which is 4: it is at least four cells long there. It is listed once, at the coarsest level where that holds, with its own bounds and the location of its payload, and it is not listed at any finer level. The directory is built over the entities that are not straddlers, so its cells stay tight. The payload is stored once, wherever the writer put it (P5.2's rule), and the list carries a reference to it.

ADR 0005 kept one answer in reserve beside the packed R-tree: leaf cells with a long-entity bucket (its model M4), which P1.6 measured as a single bucket. I split that bucket by scale, one list per level, because one bucket makes an entity that is merely wide cost what a drawing-wide one costs: both are read by every viewport that reads the bucket. With a list per level the wide entity is in a level whose pages a viewport only reads when it touches them. The factor is the one knob. 4 is my starting value, chosen so that an entity is long only when it would touch four or more cells in a row at that level, and it is a named constant so P8.6 can tune it against real drawings.

The cell size of each level is a model of the directory, not a measurement of it, because the directory has to be built after the classification and the classification needs the sizes. `classify` takes the shorter half of the entities (by longer side, ties by entity id) and builds a directory over them. The finest cell size is the lower quartile of the longer sides of that directory's leaves. As long as fewer than half of the entities are long none of them is in the shorter half, so the estimate is the size of a cell of ordinary entities however many straddlers there are and wherever they lie. I tried reading the sizes from a directory built over everything first and dropped it: on `stress-long`, with a third of the entities long and scattered, nearly every leaf is stretched and the estimate was too large to call anything long. Half the entities make cells larger by at most the square root of two than all of the local ones would, which keeps a borderline entity local rather than listing it by mistake. Each level above grows the cell by the square root of the fanout (a node covers `FANOUT` times the area of the one below), capped at the longer side of the local drawing, so a level no longer than the drawing can never be exceeded. A drawing in which most entities are long has no ordinary cell to measure. Little or nothing is then called long, which is still correct, only not useful.

What the model costs: an entity up to four cells long stays in a cell, and does stretch it by up to four cells. On `stress-long` that is the 2,032 lines between one and four cells long. The factor is what bounds the damage.

### 3.2 Layout

The straddler block is its own run of bytes with the magic `PVST`. It is not a block kind of the container yet, for the same reason P5.2's reference arrays are not (its scope note): a kind needs a CONTAINER.md entry and a decision on lazy reads, and one follow-up, #136, does it for the straddler block and #131 for the reference arrays. Until then it is built by `build_straddlers`, parsed by `StraddlerLists::parse`, and carried by whoever holds the bytes. All integers are little-endian, all floats IEEE 754 binary64, all reserved fields zero. The header is 96 bytes:

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | magic | bytes | `PVST` |
| 4 | 2 | version | u16 | 1 |
| 6 | 2 | flags | u16 | zero |
| 8 | 4 | levels | u32 | levels of the scale, 1 to 8, finest first |
| 12 | 4 | pages | u32 | page descriptors |
| 16 | 4 | entries | u32 | entries in all pages |
| 20 | 4 | reserved | u32 | zero |
| 24 | 8 | factor | f64 | the factor the entries were classified with, above 1 |
| 32 | 64 | cell_size | 8 x f64 | the cell size of each level, zero past `levels` |

Then `pages` descriptors of 40 bytes, finest level first, then the entries in the same order. A level's list is cut into pages of at most 64 entries (`STRADDLER_PAGE`), which start near each other so a page's bounds stay tight: inside a level the entries are ordered by the Hilbert key of the lower-left corner of their bounds, ties by entity id, so the same set gives the same bytes whatever order it arrives in.

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 2 | level | u16 | the level of every entry of the page |
| 2 | 2 | reserved | u16 | zero |
| 4 | 4 | count | u32 | entries in the page, 1 to 64 |
| 8 | 32 | bounds | 4 x f64 | min x, min y, max x, max y: the union of the page's entries |

An entry is 44 bytes: the entity's bounds as four f64, then its entity id, the block and the slot of its payload as three u32. The block and slot are P5.2's `PayloadRef` (eight bytes), so what its placement pass hands over goes straight in through `LongEntity::from(PlacedEntity)`. A page is 2,816 bytes at full size and its descriptor is 1.4 per cent of that.

### 3.3 What a reader validates

`StraddlerLists::parse` checks all of this before a query runs. The length is held to the header before any page or entry is allocated, so a header that claims four billion entries costs nothing.

| rule | meaning |
|---|---|
| STR-01 | the block is shorter than its header, or its length is not the header plus 40 bytes per declared page plus 44 bytes per declared entry |
| STR-02 | a magic, version, flags or reserved field that is not defined, a level count of 0 or above the height ceiling, or a factor that is not finite and above 1 |
| STR-03 | more entries declared than the artifact ceiling of the reader's limits (P4.8) |
| STR-04 | an entry or page whose bounds are not finite, in range and ordered |
| STR-05 | a page that breaks the shape: levels not ascending or not below the level count, no entries or more than a page holds, entries that do not tile the pages exactly, or bounds that are not the union of the page's entries |
| STR-06 | a level cell size that is not finite and above 0, or a size declared beyond the levels |
| STR-07 | an entry at the wrong level: its longer side must exceed the factor times the cell size of its level and must not exceed it for any coarser level |
| STR-08 | an entity id at or above the NP-18 ceiling, or an entity listed twice, in one page, in one level or in two |
| STR-09 | a block cannot be built from the input: a duplicate entity id, an entity that is no straddler at any level, or more entities than NP-18 allows |

STR-07 and STR-08 together are the structural form of "a straddler listed at level 2 is not listed again at level 5": every entry's level is a function of its own bounds and the declared scale, and no entity appears twice, so a reader can check the placement without trusting the writer.

### 3.4 Queries

`query_viewport` returns the cells `SpatialDirectory::query` yields (chapter 1.4) and the straddlers whose bounds touch the box, finest level first and in page order. A page whose bounds miss the box is skipped whole, and a reader fetches only the pages it keeps: `StraddlerLists::read_bytes` is the header, every page descriptor and the entries of the pages touched.

The issue says to union the straddler lists of every level the query descends through. I read that as every level whose list the box touches, and not only the levels where it touches a tree node, because the directory is built over the local entities alone. A long line that crosses a gap in the drawing is reachable only through its list, and a viewport in the gap matches no cell and must still find it. `gap` is the fixture for that: a viewport between its two clusters touches no cell and finds the line.

Each entity is yielded once. A straddler is listed once (STR-08), and `unique_entities` merges the entities a caller decoded from the hit cells with the straddlers, so an entity that is in both (a writer that also referenced it from a cell, P5.2) comes out where it is first met and not again.

With P5.2 the two ceilings meet. `place` hands over an entity that intersects more cells than `DuplicationPolicy` allows (16 by default), counted on a directory that already holds it. On `drawing-100k` with every entity in the directory it hands over 1,006: the 1,000 spanning lines and six more entities, none of which is over the ceiling once the lines are out of the directory. Over the directory of the local entities, which is the one the classification leaves, it hands over none. Every entity the factor lists is among those it hands over, and building the lists from `place`'s output (the entities that are long at some level) gives the same bytes as building them from `classify`'s, which the tests assert. An entity that is over the ceiling but not over the factor (a large square, say, four cells on a side, touches 25) stays in its owner's cell, as P5.2 already leaves it, and is found through that cell.

### 3.5 Congestion metrics

`congestion(dir, lists)` reports, per artifact: the local entities and the straddlers, for every level the cell size, the entities placed there, the straddlers, the pages and the bytes of its list, the references the root level carries (`root_references`, the straddlers listed at the directory's top level), the bytes of the whole block, the bytes every viewport reads of it (`index_bytes`, the header and the page descriptors), and `coarsest_bytes`: what a minimal viewport read must fetch from the coarsest level, which is the directory's root node, the block's index and one page of the coarsest level that has a list. `ancestor_placement` is the baseline: it places each entity at the lowest node of the directory whose bounds contain it, which is what promoting to the ancestor does, so a report can say what the naive answer would have put in the root.

### 3.6 Measurements

Native x64 on MARS, `rustc 1.99.1`, release, hot blocks written through the container writer as lines. All of it is asserted by `crates/pvec-stream/tests/issue_12_straddlers.rs`.

`drawing-100k`: 100,000 local entities (20 x 20 rooms of 250, 500 units wide) and 1,000 drawing-spanning lines over 10,000 x 10,000. The estimated cell sizes are 459, 2,594 and 10,000 for levels 0 to 2 (the median node side of the directory over the local entities is 486, 2,103 and 10,000). All 1,000 lines are listed at level 0, in 16 pages: the block is 44,736 bytes and its index 736.

| one 500 x 500 screen at the centre | cells | bytes |
|---|---|---|
| straddler lists | 9 | 46,369: cells 23,328, list pages 20,448 (of the 44,736-byte block), 64 payloads 2,593 |
| everything in cells (P5.1 alone) | 10 | 25,920 |
| promote to the root (cells and every long payload) | 9 | 63,829 |

The stated budget is 64 KiB and 2 per cent of the artifact, which is 4,325,685 bytes with the block and the straddlers' payloads: the straddler path is 1.07 per cent. The root's reference count is where the strategies differ most, and the stated ceiling is 64, one cell's capacity: promote-to-ancestor puts 1,000 references in the root, the lists put 0 there (1,000 at level 0).

I should be straight about what the screen row says. For this fixture the status quo is cheaper by about 20 KB at the centre of the drawing. The spanning lines' centres all lie on the middle cross, so Hilbert order puts them in cells of their own, and those cells are the only stretched ones. The strategy pays where long entities are scattered among local ones, which is what real drawings do, and `stress-long` is that case: 20,000 room entities and 10,000 random axis-aligned lines of 1,000 to 5,000 over 5,000 x 5,000. 7,968 of them are classified long (estimated cell sizes 447, 2,530, 5,000). Over 60 random 250-unit viewports the lists read 15.4 MB in 1,973 cells, and the same entities all in cells read 22.6 MB in 8,725.

Congestion metrics for each fixture of the phase 5 set (`local`, `long`, root references with the promote baseline in brackets, block bytes, index bytes, `coarsest_bytes`):

| fixture | local | long | root refs (promote) | block | index | coarsest read |
|---|---|---|---|---|---|---|
| `drawing-100k` | 100,000 | 1,000 | 0 (1,000) | 44,736 | 736 | 3,600 |
| `stress-long` | 22,032 | 7,968 | 0 (5,223) | 355,688 | 5,096 | 7,960 |
| `rooms-1m` plus 2,000 long lines | 1,000,000 | 2,000 | 0 (1,695) | 89,376 | 1,376 | 4,240 |
| `gap` | 3,600 | 3 | 0 (3) | 268 | 136 | 316 |
| `tied` (640 identical boxes) | 640 | 0 | 0 (0) | 96 | 96 | 144 |

`rooms-1m` uses two levels: 500 lines of 8,000 to 30,000 units at level 0 and 1,500 longer ones at level 1, so the once-per-level rule is exercised on a fixture with more than one list.

### 3.7 Out of scope here

Writing the block into the container and the placement of straddler payloads (#136), previews (P5.4), read planning (P5.5, P5.7), the per-entity references inside a cell (#117), tuning the factor and the page size against real drawings (P8.6), and splitting a long entity into pieces, which would change its identity and its geometry.

## 5. Viewport queries and read plans

A pan or a zoom has to become the smallest practical set of range requests: not one per cell, and not the whole file. `ViewportQuery::plan` (in `crates/pvec-stream/src/viewport.rs`, issue P5.5) turns a viewport rectangle into a `ReadPlan`: an ordered list of the range requests the container reader's planner (P4.5) builds for the blocks the viewport wants. It reads nothing and changes nothing, and it never opens a file: it takes the `SpatialDirectory` and anything that answers `PlanSource` (both `ContainerReader` and `AsyncContainerReader` do), so the same plan serves a blocking and an async reader.

### 5.1 What goes in the plan

A query is `bounds` (source space), `scale` (display pixels per source unit, finite and above zero) and `tier_budget` (bytes). The config carries `prefetch_margin_px` (default 128, zero turns it off), which is turned into source units with the scale. The directory yields the cells that touch the viewport grown by that margin, and each hot block of those cells is looked at by its own bounds, because a cell's bounds are the union of its blocks and often wider:

* a block that touches the viewport is **visible**,
* a block that touches only the margin is **prefetch**,
* anything else is not in the plan, which is how the plan never holds a block outside the viewport plus the margin (a coalesced request can still read the bytes between two blocks it holds, and that is counted as waste below),
* a block the reader's cache already holds is counted (`blocks_cached`) and not requested, which is all a pan or a zoom needs to ask only for the delta. `is_cached` looks and does not touch, so planning moves neither the LRU order nor the hit counters, and the same viewport and cache state give the same plan, which the tests assert by comparing two independently built plans.

All wanted blocks go through one call of the reader's `plan`, so adjacent blocks are merged under the gap threshold and a request never re-reads a block another request of the plan fetches. A request that holds any visible block is a visible request, and the margin blocks riding in it (across a gap, say) are charged to the margin in the metrics. A request of margin blocks alone is a prefetch request.

### 5.2 Order

Visible requests come first, the one that brings the most visible entities per byte first: a block's value is its entity count times the share of its bounds inside the viewport (per axis, so a block of horizontal lines is not worth nothing), summed over the request and divided by its length. Equal rates go in file order. Prefetch requests follow, nearest the viewport centre first. The order depends only on the plan's inputs, so it is deterministic.

### 5.3 Budget, metrics, and what the plan does not cover

`tier_budget` caps the bytes of visible requests in one plan. The first request always goes, so a query makes progress even under a budget of zero, requests past the budget move to `deferred` in the order they would have come (the prefetch with them), and `deferred_ranges` and `deferred_bytes` say what was left. P5.4 (tiers) has not landed, so there is no tier selection here: the budget applies to the exact hot blocks, and a tiered plan is that task's to add on top of this one.

`PlanMetrics` has a `ClassMetrics` each for visible and prefetch: `ranges`, `bytes_requested`, `bytes_wasted` (the request's length less the stored length of the blocks it holds, which is what the gaps cost), `blocks` and `blocks_cached`. The two classes add up to the bytes the requests span. The advanced curve set is one set in cell 0 that no leaf holds, so it is not in the plan; `advanced_in_view` says when it touches the viewport, and a cell reaches it by reference (section 2.8).

A bad query is refused by rule: VP-01 for a scale that is not finite and above zero, VP-02 for a margin that is negative, not finite or pushes the box out of range, VP-03 for a reader refusal (which carries its own rule).

### 5.4 Measurements

Native x64 on MARS, `rustc 1.99.1`, `rooms-1m` of chapter 1.5 (15,625 hot blocks of 2,592 bytes, 40,500,000 bytes in all), reader defaults (64 KiB gap, 16 MiB request ceiling, 1 GiB cache for the tests).

| criterion | stated | measured |
|---|---|---|
| full extent, complete exact coverage | at most 4 range requests | 3 requests, 40,500,000 bytes, 0 wasted |
| half-screen pan, no prefetch, viewport 25,000 wide | delta at most 0.65 of the initial view's bytes | 0.507 (1,513,728 of 2,983,392), 551 blocks from cache |
| one-screen pan with a one-screen margin already fetched | delta at most 0.1 of the initial view's bytes | 0 |
| waste at the default gap, view 25,000 and full extent | at most 0.25 of bytes requested | 0.113 and 0 (prefetch 0.002) |
| waste at the default gap, view 6,000 (about 50 blocks) | at most 0.55 | 0.494 |
| zoom into a region the outer view decoded | no request for a decoded block | 0 ranges, 0 bytes |

The last waste row is the honest limit of the fixed threshold: 64 KiB is about 25 blocks here, so a handful of scattered Hilbert runs read a lot of padding. Retuning the gap against block size is P8.6's, and the number is there so that tuning has something to beat.
