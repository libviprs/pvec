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
offset 0, the manifest at offset {header}, the blocks, and the block directory, which ends the file. Every \
block starts on a multiple of {align} bytes from the first byte of the file, the directory too, so a block \
can be mapped and read in place. The manifest holds one directory pointer, of kind 1 (the block directory), \
and declares no block checksum table and no artifact digest: P4.6 adds those, and an artifact without them \
is a valid one (section 1.4). The header and the manifest agree on where the directory pointer table \
lies, as chapter 1 requires.\n\n\
### 4.2 The block directory\n\n\
The block directory is a table of {entry} byte entries, one per block, in file order. A block carries \
its offset, its stored length, the length once decoded (equal while no codec is used), a count, its spatial \
cell, the bounds of its entities, a kind and a subtype, flags and a codec id. ADR 0003 priced the entry at \
56 bytes and ADR 0006 at 64 once the codec and the uncompressed length were added; ADR 0008's kind and \
flags fit in the same 64 bytes.\n\n",
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
    ] {
        s.push_str(&format!("| {} | {} | {} | {} | {} |\n", kind.id(), kind.name(), holds, subtype, count));
    }
    s.push_str(&format!(
        "\nFlag bit 0 is must-understand: a reader that does not know the kind refuses the file (ADR 0008). \
Geometry blocks carry it; the component stream set does not, so a reader that does not know it skips it. \
Flag bit 1 says the bounds are meaningful: a hot block and the record block carry the union of the bounds \
the caller gave their entities, an arena or a component set carries none and its four bounds are zero. The \
codec is 0 (stored as it is); P4.6 adds the others. A reader checks every entry before using it:\n\n\
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
| block kind | the blocks of the file | the kind id of section 4.2, so hot blocks come first and the component set last |\n\
| spatial cell id | the blocks of one kind | the cell id the caller gave the entity (P5.1 decides what a cell is; the writer only orders by it) |\n\
| geometry type | the blocks of one kind and cell | the hot geometry type or the payload kind |\n\
| `EntityId` | the entities inside a block | the id, so a block's id column ascends and equal sets of entities give equal columns |\n\
| capability id | the manifest's capability table | the stable id from the registry (P3.4), each declared once |\n\
| stream id | the streams of the component set | the capability id, which is the id of a stream (P3.7) |\n\n\
The advanced curves are one set of six blocks (the record block and the five arenas) in cell 0, because a \
composite names other records of its own block, so a curve cannot be split from its segments by cell; an \
advanced curve given another cell is refused. In short, cell 0 is both no cell (the advanced set and the component set) and an ordinary cell a hot block can be in, so a block in cell 0 does not say which it is: P5.1 gives the spatial directory the way to tell the two apart (the block kind does today). The records are in ascending `EntityId` order and the \
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
positive zero. The bounds `push_curve` derives for a model curve are conservative boxes from comparisons and additions only (vertices, control points, end points, or the whole circle or ellipse for an arc), because the model's own bounds of an arc go through `sin_cos` and `atan2`. The writer calls no transcendental function: a curve whose stored form would need one \
(an arc on a rotated frame, whose angle needs `atan2`) is refused (WRT-08) instead of being stored with \
a last bit that belongs to the C library of the machine.\n\n\
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
* A new field of the artifact is written from caller-supplied or derived values only.\n",
    );
    s
}
