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

**Cell 0 is reserved.** In the block directory, 0 is "no cell": the first advanced chunk, the component set and the spatial directory itself are in it. The directory therefore never uses 0 for a leaf (nor an id from `STRADDLER_CELL_FIRST` up, which is where the straddlers' payloads live, section 3.2), and a file that carries a directory is refused when a hot block is in cell 0 (SPD-10), because the entities in it would be invisible to every query. The block kind says what a block in cell 0 is, as it did before. The advanced curves are chunks the container cannot split by cell, so no leaf holds them; `SpatialDirectory::advanced_bounds` reports its bounds beside the cells, and a cell references its curves by the row of the record block instead of holding them (section 2.8).

Per-entity references with their own bounds inside a cell are not in this version: with every payload owned by one cell (ADR 0005) there is nothing to duplicate, and a per-entity list would not fit the 2 MiB open budget at a million entities. See the scope note on issue #10.

The constants are named in `crates/pvec-stream/src/directory.rs`, with P1.6's measurements beside them: `CELL_CAPACITY` 32,768 (it was 64 until P8.6 tuned it against the progressive benchmark, `docs/bench/TUNING.md`), `FANOUT` 32, the split policy `HilbertPack` (sort by the Hilbert key of the bounds centre on a 65,536 grid, ties by entity id, cut every leaf capacity), `MAX_FANOUT` 256 and `MAX_HEIGHT` 8 as the ceilings a file may declare. `build_directory` and `classify` take the leaf capacity as an argument, because the header records the capacity the file was built with: a writer passes `CELL_CAPACITY`, and the structural tests of this chapter pass the 64 their fixtures are built for.

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
| SPD-09 | a leaf cell id that is 0, is in the straddler payload cells or does not ascend strictly from one leaf to the next |
| SPD-10 | the cells and the hot blocks disagree: a hot block in cell 0 or in no leaf (the straddler payload cells are left alone), a hot block without bounds or with bounds outside its leaf, a leaf with no block, blocks of one cell that are not contiguous, or a leaf whose entity count is not its blocks' total |
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

The advanced curves are written as K chunks. A chunk is a record block and its five arenas, all in cell k for chunk k. A record block and an arena are one group in the canonical order (group, then cell, then kind, then subtype), so the six blocks of a chunk are adjacent, the record block first, and a chunk is one range. The record block is 32 bytes a row. A composite names other records of its own block, so the writer puts a composite and every curve it names in one chunk (a writer rule, asserted by a test that resolves every composite inside its own chunk). The set is whole in the sense that no curve is split and nothing is copied (ADR 0005 stands: no spatial clustering, no duplication); it is no longer one set of six blocks. Components are ordered by their lowest entity id and packed into a chunk until it holds a size in bytes of record and arena data (256 KiB by default since P8.6, `docs/bench/TUNING.md`; it was 64 KiB), so the arenas of a chunk are one range, and one component bigger than that is a chunk of its own.

A reader's resident bytes for the advanced curves are now the chunks the viewport touches times the chunk size, which is bounded by the viewport and not by N. That is what lets the plain peak-memory criterion of #17 come back, without the subtraction of `advanced_set_bytes`.

What a cell needs is to know which curves reach it, and that is what a reference is for. An advanced curve is a payload with an owner (the record block of its chunk) and a row (its position in that block, ascending by entity id, numbered from 0 in every chunk), so it is a `PayloadRef` like any other: `block_id` is the chunk's record block and `slot` is the row, and the reference block of #131 does not change. `scan_advanced` lists them chunk by chunk. The placement pass treats them as it treats a hot entity: the curve's bounds go in, a reference goes into every leaf they intersect. `read_viewport` checks a reference to a record block (OWN-08 if the row is past the end) and leaves it alone; `advanced_rows` returns `(chunk, rows)` pairs, chunks ascending, and the reader fetches each chunk named once through `ContainerReader::advanced(chunk)` (one cache entry per block, six per chunk) and picks those rows. A composite spanning two regions is referenced from both, so a viewport over one end finds it without finding a segment it never touches.

The directory entry of a record block carries the union of the bounds of its chunk's curves, so a planner can plan by chunk bounds (#129) and not only report. `SpatialDirectory::advanced_bounds` is the union over all chunks.

`SpatialDirectory::advanced_bounds` and `ReadPlan::advanced_in_view` stay, for a file that has no reference block: they say the same thing more coarsely, from the union of the chunks' boxes.

## 3. Long entities and straddlers

One 400-metre grid line or a drawing-wide hatch boundary touches nearly every cell, and both obvious answers turn the first read into the whole file. Promoting it to the lowest node that contains it puts it in the root, which every viewport reads. Duplicating a reference to it into every cell it crosses puts it everywhere. There is a third cost I care about as much: a long entity left inside a cell stretches that cell's bounds to cover it, and a stretched cell is fetched by every viewport that touches the stretched box, whether or not anything in the cell is on screen. So I keep long entities out of the cells.

### 3.1 The design

An entity is a *straddler at a level* when its longer side exceeds that level's cell size by `STRADDLER_FACTOR`, which is 4: it is at least four cells long there. It is listed once, at the coarsest level where that holds, with its own bounds and the location of its payload, and it is not listed at any finer level. The directory is built over the entities that are not straddlers, so its cells stay tight. The payload is stored once, wherever the writer put it (P5.2's rule), and the list carries a reference to it.

ADR 0005 kept one answer in reserve beside the packed R-tree: leaf cells with a long-entity bucket (its model M4), which P1.6 measured as a single bucket. I split that bucket by scale, one list per level, because one bucket makes an entity that is merely wide cost what a drawing-wide one costs: both are read by every viewport that reads the bucket. With a list per level the wide entity is in a level whose pages a viewport only reads when it touches them. The factor is the one knob. 4 is my starting value, chosen so that an entity is long only when it would touch four or more cells in a row at that level, and it is a named constant so P8.6 can tune it against real drawings.

The cell size of each level is a model of the directory, not a measurement of it, because the directory has to be built after the classification and the classification needs the sizes. `classify` takes the shorter half of the entities (by longer side, ties by entity id) and builds a directory over them. The finest cell size is the lower quartile of the longer sides of that directory's leaves. As long as fewer than half of the entities are long none of them is in the shorter half, so the estimate is the size of a cell of ordinary entities however many straddlers there are and wherever they lie. I tried reading the sizes from a directory built over everything first and dropped it: on `stress-long`, with a third of the entities long and scattered, nearly every leaf is stretched and the estimate was too large to call anything long. Half the entities make cells larger by at most the square root of two than all of the local ones would, which keeps a borderline entity local rather than listing it by mistake. Each level above grows the cell by the square root of the fanout (a node covers `FANOUT` times the area of the one below), capped at the longer side of the local drawing, so a level no longer than the drawing can never be exceeded. A drawing in which most entities are long has no ordinary cell to measure. Little or nothing is then called long, which is still correct, only not useful.

What the model costs: an entity up to four cells long stays in a cell, and does stretch it by up to four cells. On `stress-long` that is the 2,032 lines between one and four cells long. The factor is what bounds the damage.

### 3.2 Layout

The straddler block is its own run of bytes with the magic `PVST`. It is built by `build_straddlers` and parsed by `StraddlerLists::parse`, and it is block kind 7 of the container (`straddlers`, section 3.8; CONTAINER.md section 4.2). All integers are little-endian, all floats IEEE 754 binary64, all reserved fields zero. The header is 96 bytes:

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
| STR-10 | the block entry is wrong: more than one straddler block, a count that is not the entries the lists declare, or a block stored with a codec |
| STR-11 | a payload that is not where the lists say: a block that does not exist or is not a hot block, a hot block outside the payload cell of the entry's page, or a slot past the end of the block |
| STR-12 | the container refused the read (the error carries its own rule) |

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
| straddler lists | 9 | 54,144: cells 23,328, list index and pages 20,448 (of the 44,736-byte block), 64 payloads in 4 payload blocks 10,368 |
| everything in cells (P5.1 alone) | 10 | 25,920 |
| promote to the root (cells and every long payload) | 9 | 63,829 |

The stated budget is 64 KiB and 2 per cent of the artifact, which is 4,326,920 bytes for the written file, block and payload blocks included: the straddler path is 1.25 per cent. These two rows are read from that file (`the_numbers_of_section_3_6_are_measured_from_the_written_file_and_stay_in_budget` in `issue_136_straddler_block.rs`), with every byte counted by a recording source (the directory is already open), and replace the modelled 46,369 bytes (1.07 per cent) of the first version of this chapter: the difference is that a payload is fetched as the block of its page (2,592 bytes for 64 lines) and not as a row. The other rows of the table are still the model of `issue_12_straddlers.rs`, with the payload at the average hot bytes per entity. The byte counts do not depend on the architecture (the determinism goldens write the same file on x86_64 and arm64). The root's reference count is where the strategies differ most, and the stated ceiling is 64, one cell's capacity: promote-to-ancestor puts 1,000 references in the root, the lists put 0 there (1,000 at level 0).

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

Read planning (P5.5, P5.7), the per-entity references inside a cell (#117), tuning the factor and the page size against real drawings (P8.6), and splitting a long entity into pieces, which would change its identity and its geometry. The reference arrays of P5.2 are not a block yet (#131).

### 3.8 In the container (issue P5.3b)

The lists are a block of their own kind, `straddlers` (id 7; id 6 is the reference arrays of P5.2b). It is optional, so a reader that does not know it skips it and still validates the file; it is never compressed whatever the codec policy says, because a reader fetches its index in one range and each page in a range of its own; its count is the number of entities listed; its bounds are the union of their bounds; its cell is 0. The container does not parse it: `StraddlerIndex::open` does, and checks it against the block directory with the rules STR-10 to STR-12 of section 3.3.

**Where the payloads go.** A hot block has to sit in a cell of the directory (SPD-10), and a straddler is deliberately not in the directory. I gave the payloads cells the directory never traverses: cell `STRADDLER_CELL_FIRST` (0xF0000000) plus the index of the page the entity is listed in, so there is one cell, and so one hot block, for each page of the lists. The writer already orders blocks by cell, so the payload blocks are one run at the end of the hot blocks, in the order of the pages. The directory does not number a leaf that high (SPD-09) and leaves a hot block in such a cell alone (SPD-10), and nothing else about a cell of the directory changes. I rejected a single cell for all payloads: a block is the unit a reader fetches, and one block of 1,000 lines costs 52 KB to read for any one of them, where a block per page costs 2.6 KB. I also rejected a block range inside the existing cells, which would put the payloads where the cells' bounds have to grow to contain them, the very thing this chapter exists to avoid. `payload_cells` gives the cell of each entity before the file is laid out, because the pages depend on the bounds and the scale and not on where a payload ends up.

**What an older reader does.** The block is optional, so at the container layer a reader older than this change skips it and opens the file. Its spatial directory does not: `SpatialDirectory::open` of that reader refuses a file that has straddler payloads with SPD-10, `a hot block is in a cell the directory does not have`, because the payload blocks are hot blocks in cells from `STRADDLER_CELL_FIRST` up. A hot block has to sit in a cell, so I see no way to make that reader tolerate the file without changing its code, and a format break is not on the table. So "an older reader skips it" is true of the container and false of the viewport path, and a writer should emit straddler payloads only for readers that know these cells. I call that acceptable before 1.0, where there are no deployed readers, and `a_reader_older_than_this_change_skips_the_lists_but_refuses_the_payload_cells_at_spd_10` pins the outcome by modelling that one rule.

**Writer order.** Classify, build the directory over the local entities, write the local entities in their cells and each straddler in the cell `payload_cells` names, lay the file out once to learn the block ids (`scan_payloads`; hot blocks sort before the lists, so their ids do not move when the lists are added), build the lists over those ids, and write the file again with `set_straddler_lists`. P5.2's `place` over the directory of the local entities hands nothing over (section 3.4), and the test asserts it on the written file.

**Reading.** `StraddlerIndex::open` reads the 96 byte header and the page descriptors, which is all a viewport needs to choose pages, and `StraddlerIndex::query` reads the entries of the pages the box touches, neighbouring pages in one range, and checks each payload it returns against the block directory (STR-11: a hot block, in the payload cell of its page, with the slot inside it). A range read is not covered by a whole-block checksum; `validate` and `StraddlerLists::parse` over the whole block are the full check.

## 4. Tier 0: complete first-paint coverage

A reader that paints the cheap entities first leaves holes where the expensive geometry will be, and nobody looking at the screen can tell a missing wall from an absent one. So Tier 0 (issue P5.4, ADR 0007) is defined as complete: every entity of the artifact is in it at first paint, either exactly or as a bounded-error preview that names the entity it stands for. The code is `crates/pvec-stream/src/tiers.rs`.

**Previews are display fidelity only.** A preview is never geometrically or structurally exact, `Preview::is_exact` is always false, and conformance output labels it `display-fidelity` (`Fidelity::label`) and counts it apart from the exact entities (`Tier0Plan::conformance`). Nothing in this chapter claims a preview is mathematically equivalent to its entity.

### 4.1 The plan

`Tier0Plan` assigns every entity one of:

* `Exact`: cheap geometry, emitted in full. That is a full circle, a line segment or circular arc (a trimmed line or circle), and a polyline of at most 64 vertices (`is_cheap_exact`). The geometry itself stays in its hot block (chapter 2 keeps one payload per entity), and the plan names the entity by id, so Tier 0 does not copy it.
* `Preview`: everything else (ellipse arcs, splines, Bezier curves, composites, long polylines). It carries `replaces`, the entity's own `EntityId`, the `error_bound` the writer measured, the form, and the vertices.
* declared infeasible (section 4.6), which the artifact lists and does not hide.

`Tier0Builder` takes entities in any order (`add_exact`, `add_preview`, `add_curve`) and `finish` returns the plan. `Tier0Plan::build` is the same thing over an iterator of curves.

### 4.2 The coverage invariant

The set of `EntityId`s reachable from Tier 0, plus the ones declared infeasible, equals the artifact's entity set, and a gap is a writer error: `finish` returns `Tier0Error::Gap` (TIER0-01) naming the smallest missing id, an id twice is `Duplicate` (TIER0-02), an id outside the artifact is `OutOfRange` (TIER0-03). `check_coverage` checks a finished plan against the artifact's id list the same way. A reader runs the same check on a stream (section 4.4), so a file whose Tier 0 has a hole is refused, not painted with the hole.

### 4.3 Previews and their bounds

The form follows the decision of ADR 0007, per curve class. An ellipse arc gets a chord polyline. A spline, Bezier curve or composite gets its control polygon where that is measured to be within the budget, and a chord polyline otherwise. A long polyline gets a Douglas-Peucker simplification of itself, because its control polygon is itself. A control polygon is not a bound by construction (the spike's worst was 9.1 units against a budget of 5.2), so the writer never assumes a form qualifies.

The bound is measured, not derived. `measure_deviation` takes the two-sided maximum distance between the preview and the exact curve: samples of the curve, taken through `pvec-geometry` (the only evaluator), against the preview, and samples of the preview against a dense polyline through the curve. The curve is sampled at least 512 times and 128 times per piece, and at the joints of polylines and composites so a corner is never cut. The preview is measured after its `f32` quantisation. The stored `error_bound` is the measurement times 1.1, plus a sliver for rounding, rounded up to an `f32`. The sliver follows the preview's own extent (its largest offset from its `f64` origin), because the offsets are what is rounded to `f32`; until P8.6 (`docs/bench/TUNING.md`) it followed the absolute coordinate, which at UTM-scale offsets was larger than the whole budget and left every curve of `huge-coordinate-1e5` without a preview. The writer refuses a preview whose bound is over the budget (`Tier0Params::max_error`, in world units), and a consumer reads the bound from the stream and never infers it. The budget is the caller's. The default is `PREVIEW_BUDGET_PX`, 0.125 px at the fit zoom since P8.6 (it was 0.5 px, P1.7), which is `0.125 / scale` world units. I did not derive a curvature bound for rational splines, so their previews are verified by sampling, which can in principle miss a narrow excursion between samples. The 10% margin is the allowance for that, and it is not a proof.

### 4.4 The stream

All integers are little-endian. The header is 24 bytes:

| offset | size | field |
|---|---|---|
| 0 | 4 | magic `PVT0` |
| 4 | 1 | version: 1 for the stream of the whole artifact, 2 for the stream of one leaf (section 4.5) |
| 5 | 3 | zero |
| 8 | 4 | entities in the artifact |
| 12 | 4 | exact runs |
| 16 | 4 | previews |
| 20 | 4 | infeasible records |

Then the exact runs (start `u32`, length `u32`, ascending, not touching an earlier run), the previews in ascending order of the entity they replace, and the infeasible records in ascending order. A preview is `replaces` (`u32`), form (`u8`: 0 control polygon, 1 chord polyline), vertex count (`u16`, 1 to 4096), `error_bound` (`f32`), the origin (two `f64`) and the vertices as `f32` offsets from it. An infeasible record is the id and a reason (`u8`: 0 evaluator refused, 1 vertex cap, 2 bound not met).

`Tier0Plan::decode` checks every count against the bytes that remain before it allocates, refuses a stream whose runs, previews and infeasible records do not cover the entities exactly once, and refuses trailing bytes, unknown codes, and a bound that is not finite and above zero. The exact ids are held as runs, so a small stream cannot ask for an allocation the size of its entity count.

### 4.5 Replacement

A preview has no identity of its own: it is `replaces`, the id of the exact entity. `Scene` holds exactly one thing per entity. `Scene::refine(id, curve)` swaps a preview for the exact geometry, so refinement replaces and never overdraws (`drawn` is the entity count before and after). It checks the geometry against the preview's stored bound (`Mismatch`, TIER0-07, if it is further off than that) and is idempotent: refining twice with the same geometry gives the same scene (the second call reports `Unchanged`), different geometry for an entity that is already refined is `Conflict` (TIER0-08), and an entity that went out exact is never changed. Order does not matter. Tagging a replacement with a generation is section 4.8 (P5.6), and the order Tier 1 and 2 arrive in is P5.7.

**Delivery.** ADR 0007 decided that Tier 0 is built and delivered per index leaf and not as one global first pass, because a global Tier 0 is nearly the whole artifact. It is stored that way: container block kind 8 (`tier0`, `docs/CONTAINER.md` section 4.2), one block for each leaf of the spatial directory, in the leaf's cell. The entry's count is the leaf's entity count and its bounds are the leaf's. The block is the plan's stream of section 4.4 at version 2, cut to the entities of the leaf: the same header, runs, previews and infeasible records, where the header's entity count is still the artifact's (it bounds the ids) and the records cover the leaf's entities and nothing else. A version 2 stream does not have to be dense, so the coverage rule of section 4.3 becomes: every entity of the leaf is in exactly one of the three lists and no other is. Every entity of a cell is a hot entity of that leaf, so the leaves between them cover the hot entities of the artifact once. The straddlers (chapter 3) are in no leaf, so they have no Tier 0 block; a viewport gets them from their lists, and the planner leaves them out of both tiers (section 5.3).

`tier0_blocks(plan, entities, cells)` is the writer's cut: it takes the whole plan, the boxes the directory was built from and the cells it answered, and gives one `Tier0Block` for each leaf, ascending by cell, ready for `ContainerWriter::push_tier0_leaf`. `Tier0Leaves::new(entries, dir)` is the reader's check of the entries against the directory, and `Tier0Leaves::tier0(reader, leaf)` is the read: one range, the block as stored, and the leaf's plan. `Tier0Leaves::verify` also reads the leaf's hot blocks and compares the ids. The block is optional, so a reader that does not know kind 8 skips it, and a file without it is still valid: the planner then falls back to the exact blocks.

| rule | meaning |
|---|---|
| TIER0-10 | a Tier 0 block in a cell the spatial directory does not have |
| TIER0-11 | a Tier 0 block in cell 0 or in a straddler payload cell (0xF0000000 and up) |
| TIER0-12 | more than one Tier 0 block for one leaf |
| TIER0-13 | an entry whose count is not the leaf's entity count, whose bounds are not the leaf's, or whose stream covers a different number of entities than the entry says |
| TIER0-14 | the container refused to read the block |

A leaf whose stream does not cover the leaf's entity set is `Gap` (TIER0-01, an entity of the leaf has no entry) or `OutOfRange` (TIER0-03, an entry for an entity that is not the leaf's), the same rules as for the whole plan; an id twice in a leaf stream is `Duplicate` (TIER0-02), and a version 1 stream where a leaf's is expected, or the other way round, is TIER0-05. The writer refuses cell 0, a straddler payload cell and a second block for a cell (`WRT-14`). The tests are `tests/issue_149_tier0_block.rs`.

**Compression.** The block is deflate whatever the codec policy says about the other kinds (it was LZ4 until P8.6), and is stored as it is when that is not smaller (`docs/CONTAINER.md` section 4.7). Section 4.7 below has the measurement the choice rests on.

### 4.6 Infeasibility

An entity for which no bounded preview can be made is named in the plan, with its reason, and the artifact declares it. It is not dropped, and it is not counted in `tier0_entities`. The reasons, all of them:

* `EvaluatorRefused`: the exact evaluator refused the curve under the numeric policy, so there is nothing to measure a preview against.
* `VertexCap`: no form meets the budget within 4,096 vertices. An ellipse arc a million units wide at a bound of a thousandth of a unit is the test's example.
* `BoundNotMet`: a preview can be drawn but its measured deviation, after quantisation, is over the budget.

On the fixtures of P1.7 and the generated fixtures of the tests the list is **empty**: no entity is infeasible. A class is infeasible only by one of the reasons above, never by its kind, and every curve kind of the profile has a preview form.

### 4.7 Measurements

`issue_13_the_million_entity_tier0_stays_within_its_byte_and_decode_budget` builds the plan of 1,000,000 entities, 8% of them deferred (the share P1.7 measured on `million`), and reads it back. The 80,477 previews are copies of a few hundred previews verified against the evaluator, re-identified, because verifying 80,000 of them is a writer cost the test does not need to repeat. The budget is stated in the test: 14 MiB and, for decode, 250 ms in a release build (10 s in a debug build).

| | bytes | decode |
|---|---|---|
| stream, 1,000,000 entities, 80,477 previews | 12,567,975 (12.0 MiB, about 156 bytes a preview) | 13 ms, release, arm64 (Apple silicon) |

This is the stream of one global Tier 0 for the whole drawing and not the unit a reader fetches (section 4.5), and it does not include the exact geometry the plan names, which is in the hot blocks. Where the x64 numbers are, they are on the pull request.

**Per leaf, and which codec.** `measurement_million_entities_per_leaf_compressed_or_not` in `tests/issue_149_tier0_block.rs` (an `#[ignore]` measurement, release build) builds the same shape of plan, 1,000,000 entities with 8% previews of 3 to 8 vertices, cuts it by the leaves of a directory over a 1,000 by 1,000 grid of boxes (15,625 leaves of 64 entities) and, for each codec, stores every leaf, then decompresses and decodes every leaf. The leaves add up to more than the global stream (7,747,413 bytes against 6,244,333) because each carries a 24 byte header and the exact runs of a leaf are short: a leaf's ids are scattered in id order, and a run is eight bytes however short.

| codec | stored bytes | stored over raw | decompress and decode all 15,625 leaves, x64 (MARS) | per leaf | same, arm64 (Apple silicon) | per leaf |
|---|---|---|---|---|---|---|
| none | 7,747,413 | 1.000 | 7.3 ms | 0.46 us | 86 ms | 5.5 us |
| lz4 | 6,161,891 | 0.795 | 10.1 ms | 0.65 us | 78 ms | 5.0 us |
| deflate | 5,161,071 | 0.666 | 81 ms | 5.2 us | 369 ms | 23.6 us |

The byte counts are the same on both (they do not depend on the machine). The timings are one release run each, native x64 on MARS and arm64 on the MacBook, which was not idle, so read them as ratios. LZ4 stores a fifth less than raw and costs under a microsecond a leaf on x64 (about 0.2 us more than not compressing); deflate stores a third less and costs about ten times the LZ4 time, 5 us a leaf on x64 and 24 us on arm64, so a viewport of fifty leaves would spend about a quarter of a millisecond on x64 and a millisecond on arm64. The choice is LZ4, because it is the codec ADR 0006 recommends for anything served over HTTP and because the bytes that deflate saves over it (about 13% of raw) are not worth ten times the decode on a path that runs on every pan. The fixture is synthetic, so P8.3 revisits the codec against real drawings, as it does for the other kinds.


### 4.8 Cancellation and refinement generations

While a read is in flight the user keeps panning, so results come back for views that no longer exist. Two things make showing one structurally impossible, not just unlikely.

**Generations.** `Generation(u64)` is handed out by a `GenerationClock` and goes up by one on every viewport change (`advance`, which is safe to call from any thread: each caller gets a distinct, larger number). Every read plan, decode job and refinement result is started under one. A `RefinementResult` is the only thing `Scene::apply` takes, its fields are private, and both constructors (`preview`, `exact`) require a `Generation`, so a result without one does not type-check.

**The barrier.** `Scene::apply` checks two things before it touches anything. A result with `generation < scene.generation()` is dropped as `StaleGeneration`. A result whose `Refinement` is below what the entity already holds (`TIER0` preview, `TIER2` exact; an entity that went out exact in Tier 0 starts at `TIER2`) is dropped as `StaleRefinement`, even when its generation is newer, because a Tier 0 preview must never overwrite exact geometry. A dropped result leaves the scene as it was and is counted (`Scene::dropped`). `Scene::advance(g)` tells the scene about a viewport change, and applying a result of a newer generation raises it too. The scene generation never goes back. The old checks of `refine` (`Unknown`, `Mismatch`, `Conflict`) still run after the barrier, and an error is not a drop.

**Cancellation.** `GenerationClock::advance` cancels the token of the generation it replaces. A job checks its `CancelToken` at every checkpoint: `run_job` checks before each unit (a block) and again after it, so a cancelled job finishes at most `MAX_UNITS_AFTER_CANCEL` (one) unit, the one already in flight, and throws that unit's output away. An async reader calls `token.checkpoint()` after each await in the same places. `CancelMetrics` counts `jobs_cancelled` and `work_discarded`.

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

`tier_budget` caps the bytes of visible requests in one plan. The first request always goes, so a query makes progress even under a budget of zero, requests past the budget move to `deferred` in the order they would have come (the prefetch with them), and `deferred_ranges` and `deferred_bytes` say what was left. **Tiers.** `ViewportQuery::plan_tiered(dir, reader, tier0, config)` is the plan with chapter 4 in it, where `tier0` is the file's `Tier0Leaves`. Each visible leaf that has a Tier 0 block (section 4.5) contributes that block as a `Tier::Tier0` request, and the leaf's exact blocks follow as `Tier::Exact` requests, the refinement. The tiers are coalesced apart, so no request mixes them, and every Tier 0 request comes before every exact one. `tier_budget` is spent in that order: a budget that covers Tier 0 buys complete first paint of the viewport and defers all the refinement, and the always-granted first request is a Tier 0 one. A leaf that only touches the margin is prefetched through its Tier 0 block alone (the exact blocks of a margin-only leaf are not in the plan). A leaf without a Tier 0 block is planned through its exact blocks as in `plan`, and a file with none gives the untiered plan, request for request. `PlanMetrics.tier0` is the cost of the visible Tier 0 requests, apart from `visible`, which is now the exact ones.

**Straddlers and the advanced set.** A straddler has no Tier 0 block (section 4.5) and the plan holds neither its payload block nor a Tier 0 stand-in for it: its payload blocks are in cells no leaf has, and a viewport reaches it through the straddler lists (section 3.6), always exact. I did not plan them because the lists already give a viewport its straddlers by box, so planning them here would be a second path to the same blocks, and a straddler is by construction too long for a bounded preview to stand in for it. The advanced curve set is reported by `advanced_in_view` and `advanced_chunks` and not planned, as before.

`PlanMetrics` has a `ClassMetrics` each for visible and prefetch: `ranges`, `bytes_requested`, `bytes_wasted` (the request's length less the stored length of the blocks it holds, which is what the gaps cost), `blocks` and `blocks_cached`. The two classes add up to the bytes the requests span. The advanced curve chunks are in cells that no leaf holds, so they are not in the plan; `advanced_in_view` says when it touches the viewport, `advanced_chunks` names the chunks whose record block bounds touch it (the only ones to fetch with `ContainerReader::advanced`), and a cell reaches it by reference (section 2.8).

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
| full extent, complete Tier 0 (`plan_tiered`, kind 8 blocks) | at most 4 range requests | 1 request, 5,480,431 bytes (54,469 wasted), 15,625 blocks, against 40,500,000 bytes for the exact blocks |
| zoom into a region the outer view decoded | no request for a decoded block | 0 ranges, 0 bytes |

The last waste row is the honest limit of the fixed threshold: 64 KiB is about 25 blocks here, so a handful of scattered Hilbert runs read a lot of padding. Retuning the gap against block size is P8.6's, and the number is there so that tuning has something to beat.

## 6. Rasters (ADR 0018, issue pvec#220)

A raster is not in the spatial directory (chapter 1): its tiles are found through the raster's own index block, and the plan below is what turns a viewport into byte ranges. `plan_raster` in `pvec-stream` is that plan for one raster. It reads no file: it takes the grid the placement fixes (`RasterGrid`: width, height, `tile_edge`, `levels`, channels), the six coefficients of `pixel_to_source`, the viewport (`ViewportQuery`, so source-space bounds and display pixels per source unit) and a lookup from `(level, col, row)` to what the index says about that tile (`TileSlot`). The caller fetches the index block first, lazily, like the reference arrays.

### 6.1 The level

The device size of one full-resolution pixel is the Jacobian `scale * [[a, b], [c, d]]` of the placement. Its larger singular value `sigma` is how many display pixels the image's sharper direction spends on one pixel, and it is one number under a rotation, a shear or a non-uniform scale. A level that halved `k` times has pixels `2^k` times as big, so the level is the finest one that is not under-sampled: `k = floor(-log2 sigma)`, clamped to `0 .. levels - 1`, and the level is `levels - 1 - k`. With `sigma >= 1` (zoomed in past one device pixel per image pixel) the level clamps at full resolution, which is overzoom. A pure rotation changes no singular value, so it does not change the level.

### 6.2 The window

The four corners of the viewport go through the inverse of `pixel_to_source`; the pixel-space box of the four results is divided by the level's shrink and by `tile_edge`, floored, computed in `f64` and saturated to the level's `cols x rows`, so a huge or off-image viewport can neither overflow nor allocate. Under rotation the box over-fetches; the test holds it to 1.8x the exact tile set at 30 degrees, and a renderer may cull against the viewport polygon. A window past 2^20 cells (the default `LIMIT-14`, the most tiles an index can hold) is never walked: the plan goes coarser instead.

### 6.3 The budget

The plan sums `tile_edge^2 * channels` over the tiles it would hand on (stored and inline, not the ones the index says are not stored). While that is over `max_plan_decoded_bytes` (`LIMIT-16`, 256 MiB at the default, so 256 tiles of 512 px) it drops one level coarser, and if even the coarsest level does not fit, the raster is refused with `RasterPlanError::OverBudget`. A placement with a tiny scale therefore never puts every full-resolution tile into one plan.

### 6.4 The ranges

The stored tiles that are not inline are sorted by file offset and merged: a gap of at most `ReaderConfig::gap_threshold` (64 KiB) is bridged while a range stays within `max_request_bytes`. Only tiles merge with tiles. Kinds 9 and 10 sort after every geometry block in the canonical order, and a range never starts before its first tile or ends after its last, so a gap is never bridged into a hot block. A tile the index says is not stored (length 0) gives no tile and no range. An inline tile (the coarse levels kept in the index block for Tier 0) is in the plan as `TileSlot::Inline`, already present, and is not requested.
