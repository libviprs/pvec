# Component streams

Style, text, hatch and provenance evolve on a different schedule from the geometry mathematics, so they
are not geometry. Each is an independently versioned, independently skippable stream with its own registry
capability (`docs/CAPABILITIES.md`, ids 14 to 19), and none of them can move a conformance verdict. The
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

A hatch boundary kind other than 1 is refused. Kind 2 is the word an inline curve would use, and it is
refused as embedded geometry: curve parameters in a hatch stream fail validation.

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

## What this does not decide

The meaning of text and font policy (N5.5, EPIC M rendering), the hatch semantics (N5.6), the content of
provenance records (N3.2) and the block framing and compression (phase 4).
