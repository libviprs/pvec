# Security limits

This file is generated from `crates/pvec-container/src/limits.rs`. I do not edit it by hand: a test regenerates it and fails if the committed file differs by a single byte. To change it I change the constants and run `PVEC_BLESS_LIMITS=1 cargo test -p pvec-testkit --test issue_9_limits_corpus`.

A `.pvec` file may be uploaded by anyone and decoded inside a browser tab, so every count and offset in it is attacker-controlled. `Limits` is the one configurable set of ceilings. Each is enforced before the allocation or the work it protects, and a file that passes one is refused with `LimitExceeded { limit, declared, ceiling }`: a distinct error, never a panic and never a silent clamp. The reader threads an `Accountant` through every buffer it makes, so the allocation budget is the sum of the live buffers and not one allocation at a time.

The geometric ceilings cite NP-18 in `docs/NUMERIC_POLICY.md` and take their defaults from the constants of `pvec-numeric`. `Limits::validate` refuses a ceiling above NP-18's maximum, because the block parsers refuse a file past it whatever the configuration says. Lowering one is what the browser preset does; the other limits can be raised, and a test shows that raising one lets a fixture through that the default refused.

## Defaults and the browser preset

| rule | limit | field | default | browser |
|---|---|---|---|---|
| LIMIT-01 | ExpansionRatio | `max_expansion_ratio` | 64 | 32 |
| LIMIT-02 | BlockBytes | `max_block_bytes` | 268435456 | 67108864 |
| LIMIT-03 | AllocationBudget | `max_alloc_bytes` | 2147483648 | 268435456 |
| LIMIT-04 | DecodeWork | `max_decode_work` | 1099511627776 | 17179869184 |
| LIMIT-05 | ArtifactBytes | `max_artifact_bytes` | 68719476736 | 4294967296 |
| LIMIT-06 | Blocks | `max_blocks` | 1048576 | 131072 |
| LIMIT-07 | EntitiesPerArtifact | `max_entities_per_artifact` | 268435456 | 16777216 |
| LIMIT-08 | EntitiesPerBlock | `max_entities_per_block` | 268435456 | 1048576 |
| LIMIT-09 | ControlPoints | `max_control_points` | 65536 | 16384 |
| LIMIT-10 | Knots | `max_knots` | 65553 | 16401 |
| LIMIT-11 | NestingDepth | `max_nesting_depth` | 4 | 3 |
| LIMIT-12 | Extent | none | the artifact length | the artifact length |

## Rationale and where each is enforced

| rule | rationale | enforced |
|---|---|---|
| LIMIT-01 | A deflate stream can expand about 1000 times and an LZ4 block about 255 times, so an unbounded ratio is a decompression bomb. Real blocks of coordinates compress 2 to 4 times, so 64 leaves a wide margin. | the block directory, from the declared lengths, before anything is fetched or reserved |
| LIMIT-02 | A block is read and decoded whole, so its size is the largest single buffer. The directory carries 32-bit lengths; this is far below that. | the block directory, from the stored and the declared decoded length, before the block is fetched |
| LIMIT-03 | The sum of the live decoded buffers, so many blocks that are each legal cannot add up to the machine's memory. | the allocation accountant, before every decoded buffer is created; unreferenced cache entries are evicted first |
| LIMIT-04 | Time, not memory: a file that makes the reader decode the same large blocks over and over would stall a tab without allocating more than the budget. | the allocation accountant, before every block or stretch of padding is fetched |
| LIMIT-05 | The length of the artifact as the source reports it. Offsets in the file are held to it, so this bounds every range the reader will ask for. | opening, before the first range is requested |
| LIMIT-06 | The directory is fetched whole, at 64 bytes a block, so this bounds what a cold open transfers and allocates (ADR 0008 measured 0.17% of the file at 512 entities per block). | opening, from the directory pointer, before the directory is fetched |
| LIMIT-07 | Cites NP-18's maximum of entities in one file (2^28). The identity checks of a full validation hold one id per entity. | opening, from the counts in the directory, and again when the whole file is validated |
| LIMIT-08 | Cites NP-18's maximum of entities in one file, since one block may hold them all. | the directory count, and the count in the block's own header before the block is parsed |
| LIMIT-09 | Cites NP-18's maximum of control points on one curve (2^16). | reading a curve, from the offset-table entry, before anything is copied |
| LIMIT-10 | Cites NP-18's maximum of knots on one curve (control points plus degree plus one at the maxima). | reading a curve, from the offset-table entry, before anything is copied |
| LIMIT-11 | Cites NP-18's maximum composite nesting depth (4, the outermost curve counting as 1). A walk deeper than this is refused before the segments are resolved, so the recursion is bounded by the ceiling. | the nesting walk over the record block, before any segment is resolved |
| LIMIT-12 | Every offset and length in the file is attacker-controlled, and a range past the end of the artifact (or one whose end wraps around 2^64) is never fetched. The ceiling is the length of the artifact itself, so it has no configuration of its own and no browser value. | the block directory, from the raw entries, before the table is parsed or any block fetched |

## Refusal behaviour

Every row is a `LimitExceeded` carrying the rule, the value the file declared and the ceiling. For `LIMIT-03` and `LIMIT-04` the declared value is the total the request would have brought the reader to. For `LIMIT-11` it is the first level over the ceiling, since the walk stops there; a composite that leads back to itself has no depth and is refused as a cycle (`ARN-21`) instead. The reader refuses before it reserves: a test per row uses a counting allocator to show that the size the file declared was never requested.

A refusal leaves the reader usable. An accountant that refuses a reservation has reserved nothing, and the blocks already decoded keep their bytes until they are dropped.

## What the browser preset refuses that the defaults admit

Of the phase 2 corpus fixtures, these (the defaults admit all of them):

* `composites`

In general the browser preset refuses an artifact over 4 GiB, a block over 64 MiB, a directory of more than 131072 blocks, an expansion ratio over 32, more than 2^24 entities, a curve of more than 16384 control points, composites nested more than 3 deep, and a reader that would hold more than 256 MiB of live decoded blocks.
