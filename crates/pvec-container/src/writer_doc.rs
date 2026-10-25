//! The text of chapter 4 of `docs/CONTAINER.md`, rendered from the constants.
//!
//! It lives apart from `writer.rs` because it names the things the writer must never read (a clock,
//! a path, a host name), and the source scan in `tests/determinism.rs` looks for those names in the
//! writer. Nothing here is part of an artifact.

use crate::directory::{BLOCK_ALIGN, BlockKind, ENTRY_LEN};
use crate::header::Header;
use crate::writer::WRITE_RULES;

/// Chapter 4 of `docs/CONTAINER.md`, rendered from the constants.
pub(crate) fn render_chapter() -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "## 4. The writer\n\n\
The writer turns semantic geometry into one artifact, and the same input gives a byte-identical artifact: \
the same file whatever order the entities, capabilities and streams arrive in, whichever thread pushed \
them, whichever process or machine wrote it, and whenever it ran. An artifact that differed between two \
runs could not be cached by digest, could not be diffed in review and would make every fidelity report \
unreproducible. The caller decides the entity ids, the styles, the cells, the bounds, the unit, the \
producer identity and the capabilities to declare; the writer decides everything else.\n\n\
### 4.1 File layout\n\n\
The artifact is, in this order and with nothing between them but zero padding: the {header} byte header at \
offset 0, the manifest at offset {header}, the blocks, the block checksum table, and the block directory, which \
ends the file. Every block starts on a multiple of {align} bytes from the first byte of the file, the table and the \
directory too, so a block can be mapped and read in place. The manifest holds one directory pointer, of kind 1 (the \
block directory), and by default a pointer to the checksum table and the artifact digest (section 4.7); a writer \
told not to seal leaves out the table, and an artifact without it or without the digest is a valid one (section \
1.4). The header and the manifest agree on where the directory pointer table lies, as chapter 1 requires.\n\n\
### 4.2 The block directory\n\n\
The block directory is a table of {entry} byte entries, one per block, in file order. A block carries \
its offset, its stored length, the length once decoded (equal while no codec is used), a count, its spatial \
cell, the bounds of its entities, a kind and a subtype, flags, a codec id and the id of the capability it \
belongs to. ADR 0003 priced the entry at 56 bytes and ADR 0006 at 64 once the codec and the uncompressed length \
were added; ADR 0008's kind and flags, and the capability id that lets a reader skip a block it does not know from \
its descriptor alone (chapter 5), fit in the same 64 bytes.\n\n",
        header = Header::LEN,
        align = BLOCK_ALIGN,
        entry = ENTRY_LEN,
    ));
    s.push_str(&crate::spec::table(crate::directory::BLOCK_ENTRY_FIELDS));
    s.push_str(
        "\nThe block kinds:\n\n\
| id | name | holds | subtype | count |\n|---|---|---|---|---|\n",
    );
    for (kind, holds, subtype, count) in [
        (BlockKind::Hot, "a hot geometry block (chapter 2)", "the hot geometry type, 1 to 4", "entities"),
        (BlockKind::Records, "the advanced curve records (chapter 3)", "0", "records"),
        (
            BlockKind::Arena,
            "one advanced payload arena (chapter 3)",
            "the payload kind, 1 to 5",
            "offset-table entries",
        ),
        (BlockKind::Components, "the component stream set", "0", "streams"),
        (
            BlockKind::SpatialDirectory,
            "the spatial directory (`docs/STREAMING.md` chapter 1)",
            "0",
            "directory nodes",
        ),
        (
            BlockKind::CellRefs,
            "the per-cell payload references (`docs/STREAMING.md` chapter 2)",
            "0",
            "cells that have references",
        ),
        (
            BlockKind::Straddlers,
            "the straddler lists (`docs/STREAMING.md` section 3.2)",
            "0",
            "listed entities",
        ),
        (
            BlockKind::Tier0,
            "the Tier 0 stream of one index leaf (`docs/STREAMING.md` section 4.5)",
            "0",
            "entities the leaf's Tier 0 covers",
        ),
    ] {
        s.push_str(&format!("| {} | {} | {} | {} | {} |\n", kind.id(), kind.name(), holds, subtype, count));
    }
    s.push_str(&format!(
        "\nFlag bit 0 is must-understand: a reader that does not know the kind refuses the file (ADR 0008). \
Geometry blocks carry it; the component stream set, the spatial directory, the reference arrays, the straddler lists and the Tier 0 blocks do not, so a reader that does not know them skips them. \
Flag bit 1 says the bounds are meaningful: a hot block and the record block carry the union of the bounds \
the caller gave their entities, the spatial directory carries the box of its root, the straddler lists carry the union of the boxes they list, a Tier 0 block carries the bounds of its leaf, an arena or a component set carries none and its four bounds are zero. The \
codec ids are in section 4.7; a codec id the reader does not know refuses a must-understand block and skips an \
optional one. The capability id is 0 for every kind this chapter defines and, for a block of a kind the reader does \
not know, names the capability the block belongs to (chapter 5). A reader checks every entry before using it:\n\n\
| rule | meaning |\n|---|---|\n{}\n",
        crate::directory::DIRECTORY_RULES
            .iter()
            .map(|(r, m)| format!("| {r} | {m} |"))
            .collect::<Vec<_>>()
            .join("\n")
    ));
    s.push_str(
        "\n### 4.3 Canonical order\n\n\
Every order below is decided by the writer from the content, never from the order of arrival. A block \
is one group of entities that share a block kind, a spatial cell id and a geometry type; the writer does \
not split a group by size (that is tuning, P8.6), so a group is one block.\n\n\
| key | orders | ascending by |\n|---|---|---|\n\
| block kind | the blocks of the file | the kind id of section 4.2, so hot blocks come first, then the spatial directory, the reference arrays, the straddler lists and the Tier 0 blocks, when there are any, last |\n\
| spatial cell id | the blocks of one kind | the cell id the caller gave the entity (a leaf of the spatial directory, `docs/STREAMING.md` chapter 1; the writer only orders by it) |\n\
| geometry type | the blocks of one kind and cell | the hot geometry type or the payload kind |\n\
| `EntityId` | the entities inside a block | the id, so a block's id column ascends and equal sets of entities give equal columns |\n\
| capability id | the manifest's capability table | the stable id from the registry (P3.4), each declared once |\n\
| stream id | the streams of the component set | the capability id, which is the id of a stream (P3.7) |\n\n\
The advanced curves are written as K chunks, each a record block and its five arenas (six blocks), and chunk k is in cell k, so the cells of the records and arenas number the chunks and are not cells of the spatial directory. A composite names other records of its own block, so the writer puts a composite and every curve it names, directly or through other composites, in one chunk; a component is never split and nothing is copied, and a component bigger than the chunk size is a chunk of its own. Components are ordered by their lowest entity id and packed into a chunk until it holds the chunk size in bytes of record and arena data (64 KiB by default, `ContainerWriter::advanced_chunk_bytes`), so a file whose curves fit is one chunk, in cell 0. The rows of a chunk are numbered from 0 in ascending `EntityId` order, and `PayloadRef { block_id, slot }` names the chunk's record block and the row in it. The blocks keep the canonical order of kind, then cell, then subtype, so the five arenas of a chunk are adjacent (one range) and its record block, 32 bytes a row, is in the run of record blocks; the entry of a record block carries the union of the bounds of its curves. An advanced curve given a spatial cell is refused. A cell can still reference a curve by the row of its chunk's record block (`docs/STREAMING.md` section 2.8), so a reader knows which chunks a cell needs. Cell 0 means no cell: it is the cell of the first advanced chunk, the component set, the spatial directory and the reference arrays, and the spatial directory reserves it, so no leaf of the directory is cell 0 and a file that carries a spatial directory is refused if a hot block is in cell 0 (`SPD-10`). A hot block written without a spatial directory may still be in cell 0, and the block kind says what a block in cell 0 is. The payloads of the straddler lists are hot blocks too, but no leaf of the directory holds them: they are in the cells from `STRADDLER_CELL_FIRST` (0xF0000000) up, one cell for each page of the lists, and the spatial directory neither numbers a leaf that high (`SPD-09`) nor counts a hot block in such a cell against its leaves (`SPD-10`). The straddler lists are stored as they are, whatever codec the policy names for the other kinds, so a reader fetches their index in one range and a page in another. A reader older than this change skips the lists at this layer, since the block is optional, but its spatial directory refuses a file that has straddler payloads at `SPD-10` (a hot block in a cell the directory does not have), so a writer emits them only for readers that know these cells. A Tier 0 block is in the cell of its leaf, one block for each leaf, so the blocks of kind 8 ascend by leaf id; cell 0, a straddler payload cell and a second block for one cell are refused by the writer (`WRT-14`). The block is compressed with LZ4 whatever codec the policy names for the other kinds, and stored as it is when that is not smaller (section 4.7). The records are in ascending `EntityId` order and the \
arenas number their entries in the order the writer first needs them, which is the same order, so shared \
payloads land on the same entry whatever order the curves were pushed in (section 3.6). The capabilities \
are those the caller listed plus those of the streams pushed, each once, and each is declared with the \
version and requirement the registry gives it; the manifest's minimum reader is the highest the \
declared capabilities ask for, and at least 1.0.0.\n\n\
Every scalar of every block, the directory and the manifest is written with an explicit little-endian \
conversion, so nothing depends on the byte order of the host that wrote it. The tests write the same \
input on x86_64 and on arm64 and compare the digests with a committed golden file.\n\n\
### 4.4 Stable metadata\n\n\
The producer id, the producer version and the corpus revision are exactly what the caller gives the \
writer, and there is no timestamp. No absolute path, host name, user name, environment variable, \
process id, hash-map iteration order or address-derived value is read, so a file written from another \
directory, host or time zone is the same file. Bounds are normalised so that negative zero is written as \
positive zero. The bounds `push_curve` derives for a model curve are conservative boxes from comparisons and additions only (vertices, control points, end points, or the whole circle or ellipse for an arc), because the model's own bounds of an arc go through `sin_cos` and `atan2`. The writer calls no libm function. The angle of a rotated circle or ellipse frame is the angle of its x axis by `pvec_numeric::detmath::atan2`, which is fdlibm's arctangent written with `+ - * /` and nothing else, so it is within one ulp of the true angle and the same on every machine. A rotated circle is stored as the arc that starts on its axis (the start is the rotation plus the model's start, brought into `[0, 2 pi)`), and a rotated ellipse keeps the rotation as the first of its three angles; the box of a rotated ellipse is the circle of its larger semi-axis. `ContainerWriter::derived` lists each stored rotation with a bound on its error, so a caller can report it. A frame along x derives nothing. A curve whose stored form would still need a libm function is refused (WRT-08) instead of being stored with a last bit that belongs to the C library of the machine.\n\n\
### 4.5 Refusals\n\n\
Each refusal is a typed `WriteError` with a rule identifier.\n\n\
| rule | meaning |\n|---|---|\n",
    );
    for (r, m) in WRITE_RULES {
        s.push_str(&format!("| {r} | {m} |\n"));
    }
    s.push_str(
        "\n### 4.6 Review checklist\n\n\
A change to the writer, the directory or a block builder is reviewed against these, and the \
`determinism` tests and the source scan in `tests/determinism.rs` enforce them:\n\n\
* No wall-clock time, no `SystemTime` or `Instant`, reaches the output.\n\
* No absolute path, no current directory, no environment variable and no hostname reaches the output.\n\
* No hash-map iteration order reaches the output: a `HashMap` or `HashSet` may be looked up, never iterated.\n\
* No pointer value or address reaches the output, and no pointer is formatted.\n\
* Every scalar is written with `to_le_bytes` (or its sibling), never with a native-endian conversion.\n\
* No transcendental function runs on the write path, because its last bit is the platform's.\n\
* A new canonical-ordering key gets a `SortKey`, a fixture where it matters and a mutation test that goes \
red without it; a key with no mutation test is not covered.\n\
* A new field of the artifact is written from caller-supplied or derived values only.\n\n\
### 4.7 Compression and integrity\n\n\
Blocks compress independently, so that a reader that wants one block decompresses one block, and each \
directory entry says how: its codec id, its stored length and its decoded length. The codec may differ per \
kind of block and per file, and an uncompressed block (codec 0) is always legal.\n\n\
| id | codec | stream |\n|---|---|---|\n\
| 0 | none | the block as it is |\n\
| 1 | lz4 | an LZ4 block with no size prefix; the decoded length is the entry's |\n\
| 2 | deflate | raw deflate, level 6, with no zlib or gzip wrapper |\n\n\
The writer takes a codec per kind of block (`WriterOptions`), and stores a block compressed only when that is \
smaller than it was, so a block that does not shrink stays at codec 0. The Tier 0 blocks (kind 8) are the one kind whose codec the policy does not choose: they are always LZ4, because a leaf's stream is fetched whole in one range and the measurement of `docs/STREAMING.md` section 4.7 (a million entities, 15,625 leaves) stored 0.795 of the raw bytes with LZ4 at under a microsecond a leaf against 0.666 with deflate at about ten times the LZ4 decode time. Both codecs are pure Rust, and the same \
input gives the same bytes for the version of the codec crate that `Cargo.lock` pins and the same pointer width (LZ4's \
match finder differs on 32-bit targets, so a wasm32 writer's LZ4 is not a 64-bit writer's, and deflate has no such \
difference); a compressed artifact is canonical for that version and width, and a change of codec version is a change \
of artifact. A reader holds the \
decoded length to the expansion ratio of `docs/SECURITY_LIMITS.md` before it reserves it, decompresses into exactly that \
length, and refuses a stream that has more to write (`CODEC-01`), ends short (`CODEC-02`) or is not valid (`CODEC-03`).\n\n\
Two levels of integrity, from ADR 0006. The block checksum table is one XXH3-64 of the stored bytes of each block, \
little-endian, eight bytes per block, in directory order, at the offset and length the manifest gives; it is verified \
when a block is fetched and before the block is decompressed or parsed, so a corrupt block never reaches either. \
The artifact digest is the SHA-256 of the whole file with its own 32 bytes in the manifest read as zero, so it \
covers the header, the manifest, every block, the table, the directory and the padding, and a change to any of them \
is seen, including to the ones no block checksum covers. A reader verifies it on request by reading the file in chunks of \
at most 1 MiB, not at every open. The refusals:\n\n\
| rule | meaning |\n|---|---|\n",
    );
    for (r, m) in crate::compress::CODEC_RULES.iter().chain(crate::integrity::INTEGRITY_RULES) {
        s.push_str(&format!("| {r} | {m} |\n"));
    }
    s.push_str(
        "\nThe measurement behind the codec ids and the decision on the writer's default is \
`docs/spikes/04-6-compression.md`.\n",
    );
    s
}
