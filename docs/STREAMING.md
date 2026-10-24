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

## 3. Long entities and straddlers

One 400-metre grid line or a drawing-wide hatch boundary touches nearly every cell, and both obvious answers turn the first read into the whole file. Promoting it to the lowest node that contains it puts it in the root, which every viewport reads. Duplicating a reference to it into every cell it crosses puts it everywhere. There is a third cost I care about as much: a long entity left inside a cell stretches that cell's bounds to cover it, and a stretched cell is fetched by every viewport that touches the stretched box, whether or not anything in the cell is on screen. So I keep long entities out of the cells.

### 3.1 The design

An entity is a *straddler at a level* when its longer side exceeds that level's cell size by `STRADDLER_FACTOR`, which is 4: it is at least four cells long there. It is listed once, at the coarsest level where that holds, with its own bounds and the location of its payload, and it is not listed at any finer level. The directory is built over the entities that are not straddlers, so its cells stay tight. The payload is stored once, wherever the writer put it (P5.2's rule), and the list carries a reference to it.

ADR 0005 kept one answer in reserve beside the packed R-tree: leaf cells with a long-entity bucket (its model M4), which P1.6 measured as a single bucket. I split that bucket by scale, one list per level, because one bucket makes an entity that is merely wide cost what a drawing-wide one costs: both are read by every viewport that reads the bucket. With a list per level the wide entity is in a level whose pages a viewport only reads when it touches them. The factor is the one knob. 4 is my starting value, chosen so that an entity is long only when it would touch four or more cells in a row at that level, and it is a named constant so P8.6 can tune it against real drawings.

The cell size of each level is a model of the directory, not a measurement of it, because the directory has to be built after the classification and the classification needs the sizes. `classify` takes the shorter half of the entities (by longer side, ties by entity id) and builds a directory over them. The finest cell size is the lower quartile of the longer sides of that directory's leaves. As long as fewer than half of the entities are long none of them is in the shorter half, so the estimate is the size of a cell of ordinary entities however many straddlers there are and wherever they lie. I tried reading the sizes from a directory built over everything first and dropped it: on `stress-long`, with a third of the entities long and scattered, nearly every leaf is stretched and the estimate was too large to call anything long. Half the entities make cells larger by at most the square root of two than all of the local ones would, which keeps a borderline entity local rather than listing it by mistake. Each level above grows the cell by the square root of the fanout (a node covers `FANOUT` times the area of the one below), capped at the longer side of the local drawing, so a level no longer than the drawing can never be exceeded. A drawing in which most entities are long has no ordinary cell to measure. Little or nothing is then called long, which is still correct, only not useful.

What the model costs: an entity up to four cells long stays in a cell, and does stretch it by up to four cells. On `stress-long` that is the 2,032 lines between one and four cells long. The factor is what bounds the damage.

### 3.2 Layout

The straddler block is its own run of bytes with the magic `PVST`. It is not yet a block kind of the container: wiring it into the writer and reader waits for P5.2's placement pass, which decides where the payloads and their references live, and is tracked in the follow-up issue linked from #12. Until then it is built by `build_straddlers`, parsed by `StraddlerLists::parse`, and carried by whoever holds the bytes. All integers are little-endian, all floats IEEE 754 binary64, all reserved fields zero. The header is 96 bytes:

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

An entry is 44 bytes: the entity's bounds as four f64, then its entity id, the block and the slot of its payload as three u32. The block and slot are the shape of P5.2's `PayloadRef` (eight bytes), held as two numbers until that type exists. A page is 2,816 bytes at full size and its descriptor is 1.4 per cent of that.

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

Writing the block into the container and the placement of straddler payloads (P5.2's pass, and a follow-up issue), previews (P5.4), read planning (P5.5, P5.7), the per-entity references inside a cell (#117), tuning the factor and the page size against real drawings (P8.6), and splitting a long entity into pieces, which would change its identity and its geometry.
