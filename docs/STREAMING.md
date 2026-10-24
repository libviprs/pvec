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

**Cell 0 is reserved.** In the block directory, 0 is "no cell": the advanced curve set, the component set and the spatial directory itself are in it. The directory therefore never uses 0 for a leaf, and a file that carries a directory is refused when a hot block is in cell 0 (SPD-10), because the entities in it would be invisible to every query. The block kind says what a block in cell 0 is, as it did before. The advanced curve set is one set the container cannot split by cell, so no leaf holds it; `SpatialDirectory::advanced_bounds` reports its bounds beside the cells, and splitting it by cell stays open (see the scope note in chapter 2).

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

`CellRefs` holds, per cell that has any, its references in ascending order. It has its own byte layout (`encode` and `parse`), checked like any untrusted input: the length is held to the header before anything is allocated. All integers are little endian. A reference array is not a container block yet, see the scope note below.

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

### 2.4 The duplication report and what it measured

`DuplicationReport` is the accounting per artifact: references per entity (mean, p99 by nearest rank, max), the entities that got references and the straddlers that did not, total reference bytes, and reference bytes as a fraction of payload bytes, where payload bytes are the decoded length of the hot blocks (`payload_bytes`).

Native x64 on MARS, `rustc 1.99.1`, debug build, the fixtures of `tests/issue_11_payload_ownership.rs`, default ceiling 16:

| fixture | entities | straddlers | references | mean | p99 | max | reference bytes | payload bytes | fraction |
|---|---|---|---|---|---|---|---|---|---|
| rooms (8 x 8 rooms of 400 lines) | 25,600 | 0 | 97,203 | 3.80 | 6 | 8 | 777,624 | 1,036,800 | 0.750 |
| stress-long (rooms 6 x 6 plus 24 long polylines) | 10,255 | 569 | 90,460 | 8.82 | 16 | 16 | 723,680 | 440,200 | 1.644 |
| identical (640 equal boxes) | 640 | 0 | 6,400 | 10.00 | 10 | 10 | 51,200 | 25,920 | 1.975 |
| nine-cells (575 lines and one long polyline) | 576 | 0 | 1,186 | 2.06 | 3 | 9 | 9,488 | 23,416 | 0.405 |

The stated bound, asserted by the test, is a fraction of **2.0**. That is a loose bound and the table says why: the P5.1 cells are small by entity count but not by extent, because a long entity widens its cell (section 1), so cell bounds overlap heavily and a short line reaches 3.8 cells on average in `rooms`. A line is also the smallest payload (40 bytes), so an 8 byte reference is a fifth of it. How the ceiling trades references for straddlers on the same fixtures (fraction, with the straddler count in brackets):

| ceiling | rooms | stress-long | identical | nine-cells |
|---|---|---|---|---|
| 1 | 0.002 (25,298) | 0.001 (10,758) | 0 (640) | 0.018 (524) |
| 2 | 0.043 (22,673) | 0.023 (10,172) | 0 (640) | 0.321 (80) |
| 4 | 0.508 (5,881) | 0.147 (8,209) | 0 (640) | 0.402 (1) |
| 8 | 0.750 (0) | 0.400 (6,020) | 0 (640) | 0.402 (1) |
| 16 | 0.750 (0) | 1.644 (569) | 1.975 (0) | 0.405 (0) |

Two things follow and are left for the follow-up (see the scope note). In the P5.1 tree an owning cell's bounds already contain its entity, so any viewport that sees an entity touches its owner: the references are redundant for visibility today, and they earn their bytes only for a reader that wants a cell's whole contents without walking the owners of its long entities. And the numbers should be taken again after P5.3, when straddlers stop widening the cells.

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

### 2.7 Scope note

Delivered here: the reference type and its size budget, the placement pass with the ceiling and the straddler hand-off, the report, the reference arrays with their byte layout, and read-side deduplication. Not delivered, and moved to a follow-up issue: a container block kind that stores the arrays (the layout above is ready for one; the kind needs a CONTAINER.md entry and a decision on lazy per-cell reads that #117 asks for), splitting the advanced curve set by cell (it is one set in cell 0, so it has no cell to reference from), and re-measuring and tightening the stated fraction once P5.3 lands.
