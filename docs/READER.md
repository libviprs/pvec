# The reader

This file is generated from `crates/pvec-container/src/reader.rs` and `engine.rs`. I do not edit it by hand: a test regenerates it and fails if the committed file differs by a single byte. To change it I change the constants and run `PVEC_BLESS_READER=1 cargo test -p pvec-container --test reader_doc`.

A `.pvec` file arrives from object storage over HTTP ranges and from a browser cache, and it may be hostile. One reader serves a local file, memory a caller has mapped, a positional-read interface and async HTTP ranges, and it never panics on what it is given: a refusal is a `ReadError` that names its rule. The limits it enforces are in [SECURITY_LIMITS.md](SECURITY_LIMITS.md); the layout it reads is [CONTAINER.md](CONTAINER.md).

## The checks exist once

`engine.rs` holds every check and does no IO. Each piece says which range it needs next and is fed the bytes that came back, so `ContainerReader` (blocking) and `AsyncContainerReader` (async) are the same loop around the same code. The tests drive both through one shared body and compare what they return, request by request.

## Opening a file

Three ranges at most are needed to read one block: the first 4 KiB, which holds the header and, in every file the writer produces, the manifest; the block directory, which ends the file; and the block. A manifest that does not fit the first 4 KiB is one more request. Each range is checked before the next is asked for: the header against the artifact length, the manifest against the header, the directory pointer against the manifest, the limits before the directory is fetched, and every raw directory entry (extent, expansion ratio, block size, entity count) before the table is parsed or any block is fetched.

A source that answers with another number of bytes than were asked for is an error and is never decoded, whatever its own contract says. A `206` response is the only HTTP answer accepted, and its body must be the bytes asked for.

A file can be rewritten while a reader holds its directory (a re-calibration replaces the last block, and an upload replaces the object at the same URL). `HttpRangeSource` keeps the first strong validator a response carries (its `ETag`, or its `Last-Modified` when it has no `ETag` at all, as the transport reports them through `HttpTransport::fetch_if_range`) and sends it as `If-Range` with every later range. A server answers a changed object with a `200`, which is `SourceError::Changed`; so is a `206` under another validator, from a server that ignores `If-Range`, and a `Content-Range` total other than the length the source was opened with. Nothing from the new object reaches the decoder, and the caller reopens. A weak `ETag` is never sent, because `If-Range` takes a strong validator only (RFC 9110 13.1.5). A transport that implements only `fetch` keeps the old behaviour and gets no change check beyond the `Content-Range` total, and a host behind `PvecReader::open_ranges` does the same on its side of the callback. The validator comes from the first range, not from the `HEAD` that gave the length, so a rewrite at the same length between the two goes unseen; a caller that cares opens straight after its `HEAD`, or checks the first range's `ETag` against it.

## The read plan

`plan(&[BlockId])` returns the requests that fetch the blocks, in file order, each with the blocks it covers. Two blocks whose gap is at most `gap_threshold` (64 KiB by default) are one request, the gap read and thrown away, so two blocks 512 bytes apart are fetched together and two blocks 8 MiB apart are not. A request never spans more than `max_request_bytes` (16 MiB by default) unless one block is longer than that, which is then a request of its own. `read_blocks` fetches through the plan and returns the blocks in the order asked for.

## The cache

Decoded blocks are cached by `BlockId` under a byte budget the caller supplies (`cache_bytes`, 64 MiB by default; zero turns it off). The policy is least recently used first out, each entry costing its decoded length, and a block larger than the whole budget is not kept. Dropping an entry drops the cache's reference only: the bytes go back to the allocation accountant when the last holder of the block lets go. When a new block would pass the allocation budget the cache gives up its least recently used blocks that nobody else holds, one at a time, until the block fits, and the read is refused when there are none left to give: evicting a block a caller still holds frees nothing.

## Memory

Every buffer is reserved with the accountant before it is made, the stored bytes of a block while it is fetched and its decoded bytes for as long as it lives, so the allocation budget is the sum of what is live. The prefix, the directory and the padding a validation reads are charged too, and so is the work: every byte fetched or decoded counts against `max_decode_work`. The size a file *declares* is never what is requested: a block of a declared 4 GiB is refused from its directory entry, before a byte of it is fetched.

## Validation

`validate()` reads everything except what was skipped and holds the file to the canonical layout the writer produces (the artifact digest is checked by `verify_artifact_digest`, which is separate): the bytes between blocks are zero, every block parses, the directory and the blocks agree, identities are unique, and composites are acyclic and shallow enough. It streams: blocks are decoded one at a time, padding is read a megabyte at a time, and everything it keeps (one id per entity to compare identities, the list of what to fetch, the advanced blocks until the curves have been checked) is reserved with the accountant like the blocks are.

## Rules

A refusal carries the rule of the parser that made it (`DIR-nn`, `HOT-nn`, `ARN-nn`, `STREAM-nn`, or the name of the `ContainerError` variant), the rule of a limit (`LIMIT-nn`, see SECURITY_LIMITS.md), a codec, integrity or negotiation rule (`CODEC-nn`, `INTEGRITY-nn`, `NEGOTIATE-nn`, all in chapters 4 and 5 of CONTAINER.md), or one of these:

| rule | meaning |
|---|---|
| LAYOUT-01 | the manifest does not start straight after the header |
| LAYOUT-02 | the capabilities are not strictly ascending by id |
| LAYOUT-03 | the manifest does not hold exactly one block directory pointer, or the directory does not end the file (bytes after it are refused) |
| LAYOUT-04 | the blocks are not in strictly ascending (kind group, cell, kind, geometry type) order, where a record block and an arena are one group |
| LAYOUT-06 | the directory has no blocks |
| LAYOUT-07 | a block overlaps what comes before it, or the blocks run into the directory (the parsers refuse every file that could reach this; it is kept so that removing their checks does not remove it) |
| LAYOUT-08 | a byte between the manifest, the blocks and the directory is not zero |
| CONSIST-01 | a directory entry disagrees with the hot block it describes (geometry type or entity count) |
| CONSIST-02 | the record count of the directory differs from the record block, or the entities found differ from the directory's counts |
| CONSIST-03 | an entity id is in two blocks |
| CONSIST-04 | some of the six advanced blocks are present and some are not |
| SKIPPED | a block that was skipped when the file was opened (an optional block of a kind, a codec or a capability this reader does not support) was asked for; it is never fetched, but `read_stored` returns its bytes for a tool that carries it as it is |
| SOURCE | the source failed, or answered with another number of bytes than were asked for |
| BLOCK-ID | a block id that is not in the directory |
| LIMITS | the configured limits ask for more than the format allows |

The reference reader in `pvec-testkit` also has `LAYOUT-05`, a range that lies outside the file. This reader never reaches it, because every extent is held to the artifact length first (`LIMIT-12`).

## The extent of a drawing with rasters

The extent a reader reports for fit-to-view is the box of the entities grown by the footprint of each raster's `visible_box` in source space (`pvec_stream::reported_extent`, ADR 0018). The spatial directory's root box stays a union of entities: a raster is in no leaf, and its tiles are found through its own index block (`docs/STREAMING.md`, chapter 6), not through the directory.

## Not here

There is no memory-mapped source: a file cannot be mapped from safe Rust, and the only exception the workspace allows is the confined casts in `blocks/aligned.rs`. A caller that maps a file passes the slice to `MemorySource` and gets the same checks. The file sources are in the `pvec-sources` crate, because `pvec-container` is an API crate and opens no file. Compression, the block checksums and the artifact digest are in chapter 4.7 of CONTAINER.md, and skipping what the reader does not support is chapter 5.
