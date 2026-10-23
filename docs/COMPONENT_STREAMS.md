# Component streams

Style, text, hatch, provenance and the semantic class of what is drawn evolve on a different schedule from the geometry mathematics, so they
are not geometry. Each is an independently versioned, independently skippable stream with its own registry
capability (`docs/CAPABILITIES.md`, ids 14 to 20), and none of them can move a conformance verdict. The
implementation is `crates/pvec-components`. This is the layout of the streams, version 1. The block
framing, compression and negotiation around a stream set belong to phase 4.

## What a stream is

* **Optional by construction.** A reader that implements none of them still reaches display fidelity for
  linework. The registry marks every component optional and refuses a file that declares one required.
* **Its own capability and version.** The id and the semantic version come from the registry. Bumping the
  version of a stream (`Registry::bump_component_version`) changes nothing else: the versions of the
  geometry capabilities are not touched, and a test asserts it.
* **By reference, never by value.** A stream names entities by `EntityId` (`docs/ENTITY_ID.md`) and never
  embeds geometry. A hatch is a list of boundary entities plus its own fill parameters; it introduces no
  curve type into the geometry oracle.
* **Not a geometry capability.** A stream that declares a geometry capability id is refused, on the way
  out and on the way in.

## The stream set

All integers and floats are little endian. A set starts with a 16 byte header:

| offset | size | field |
|---|---|---|
| 0 | 4 | magic `PVCS` |
| 4 | 2 | version, 1 |
| 6 | 2 | flags, must be 0 |
| 8 | 4 | number of frames, at most 4096 |
| 12 | 4 | reserved, must be 0 |

Then one frame per stream, each starting on a multiple of 8 bytes:

| offset | size | field |
|---|---|---|
| 0 | 4 | capability id |
| 4 | 6 | semantic version, three u16 (major, minor, patch) |
| 10 | 2 | reserved, must be 0 |
| 12 | 4 | payload length |
| 16 | n | the payload, then zero padding to a multiple of 8 |

A frame says what it is and how long it is, which is all a reader needs to step over it. A reader never
looks inside the payload of a stream that is unknown (the registry has no such capability), that it does
not implement, or that was written at a different major version of the stream format. Each of those is a
**report** (`Skipped`, with the reason), never an error, and the geometry opens as usual.

The reader refuses the set on: a bad magic, version or reserved field, a count above the limit or above
what the bytes can hold, a payload length past the end of the bytes, non-zero padding, trailing bytes and
a geometry capability id. Every length is compared with the bytes really present before it is used.

## The payloads

Each is consumed exactly. Strings are a u32 byte length and UTF-8 bytes, at most 255 bytes for a name and
4096 for text. Every `EntityId` is below the NP-18 limit and every float is finite.

| id | stream | payload |
|---|---|---|
| 14 | `layer` | name count, assignment count, the names, then pairs of an entity and a layer index |
| 15 | `line_style` | style count, assignment count, styles (name, dash count at most 16, dashes, each finite and not negative), then pairs of an entity and a style index |
| 16 | `text` | record count, then per record: entity, x, y, height (positive), rotation, text |
| 17 | `hatch` | record count, then per record: boundary kind (1, references), boundary count (1 to 4096), the entity ids, fill kind (1 solid with an RGBA word, 2 pattern with a name, an angle, a positive scale and an RGBA word) |
| 18 | `source_provenance` | record count, then per record: entity, source kind (u16 in a u32), handle length (at most 64), handle bytes |
| 19 | `dwg_semantics` | opaque: reserved space for future DWG semantics, never interpreted here |
| 20 | `semantic_class` | element entries, then region records, see below |

A hatch boundary kind other than 1 is refused. Kind 2 is the word an inline curve would use, and it is
refused as embedded geometry: curve parameters in a hatch stream fail validation.

## The semantic class stream (20)

What a drawn thing is in the building, so a reader can show only the walls, hide the columns or pick a room.
It is one optional stream with two kinds of subject, and it says nothing about how anything is drawn.

The payload is two u32 counts (elements, regions), the element entries, then the region records.

* **Element entry**, 24 bytes: entity, class, source, confidence present (0 or 1), confidence (f64). The
  class is 0 `Other` (so absence is stated, not implied), 1 `Wall`, 2 `Column`. An entity is classified at
  most once. The enum is closed for this stream version: a new class bumps the stream version, and a value
  this version does not define is refused (COMP-11), never carried as a vendor value.
* **Region record**: kind (1 `Room`, 2 `Corridor`), source, confidence present, confidence, then the outer
  ring, the hole count (at most 256) and each hole. A ring is a ring kind word (1, references), a count
  (1 to 4096) and that many entity ids. The ring is ordered. A region is not drawn linework, so it holds
  references and no curve parameters: ring kind 2, the word an inline curve would use, is refused as
  embedded geometry (COMP-10), in the outer ring and in a hole alike.
* **Source**: 1 `layer-rule`, 2 `block-rule`, 3 `inferred`, 4 `explicit`. Only an `inferred` entry may carry a
  confidence (finite, 0 to 1). With no confidence the confidence bytes are zero.

Decoding checks the shape. Whether the references mean anything needs the geometry, which this crate does
not depend on, so `SemanticClassStream::validate` takes an `EntityGeometry` (start and end point of an
entity, or nothing when it does not exist) from the caller and a closure tolerance. It refuses a reference
to a missing entity (COMP-16) and a ring that does not close (COMP-17). A ring closes when its entities, in
order and each in either direction, meet end to start and the last returns to where the first began. A
single entity closes when its own ends meet.

`entities_of(class)` and `regions_of(kind)` filter the stream. The spatial query (P7.5) will use them to
query by class.

## Rules a reader refuses on

| rule | meaning |
|---|---|
| STREAM-01 to STREAM-10 | the framing: magic, version, reserved, truncation, count, padding, trailing bytes, a geometry capability id, a non-component id, an oversized payload |
| COMP-01 | the payload ends before its structure |
| COMP-02 | bytes remain after the structure |
| COMP-03 | a count above its limit |
| COMP-04 | a string or byte run above its limit |
| COMP-05 | a count that must not be zero is |
| COMP-06 | a number that is not finite, or not positive where it has to be |
| COMP-07 | invalid UTF-8 |
| COMP-08 | an entity id at or above the limit |
| COMP-09 | a layer or style index with no entry |
| COMP-10 | a hatch that embeds geometry |
| COMP-11 | an undefined tag |
| COMP-12 | a reserved field is not zero |
| COMP-13 | a capability the registry does not know |
| COMP-14 | an entity classified twice |
| COMP-15 | a confidence on an entry that is not inferred |
| COMP-16 | a region or element that references a missing entity (validation) |
| COMP-17 | a region ring that does not close (validation) |

## What this does not decide

The meaning of text and font policy (N5.5, EPIC M rendering), the hatch semantics (N5.6), the content of
provenance records (N3.2) and the block framing and compression (phase 4).
