# Viewport streaming

How a reader decides which bytes of a `.pvec` file a viewport needs, and reads only those. Chapter 1 is the spatial directory (issue P5.1). Later chapters (payload ownership and references, straddlers, tiers, read planning) build on it. The code is the `pvec-stream` crate; the container underneath is `docs/CONTAINER.md`.

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

**Cell 0 is reserved.** In the block directory, 0 is "no cell": the advanced curve set, the component set and the spatial directory itself are in it. The directory therefore never uses 0 for a leaf, and a file that carries a directory is refused when a hot block is in cell 0 (SPD-10), because the entities in it would be invisible to every query. The block kind says what a block in cell 0 is, as it did before. The advanced curve set is one set the container cannot split by cell, so no leaf holds it; `SpatialDirectory::advanced_bounds` reports its bounds beside the cells, and splitting it is P5.2's.

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

Payload ownership of long entities and the reference list (P5.2), straddlers and the long-entity bucket ADR 0005 keeps as a safety valve (P5.3), tiering (P5.4), read planning and prioritisation (P5.5, P5.7), and tuning the constants (P8.6).
