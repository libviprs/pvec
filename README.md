# pvec

`.pvec` is a spatial streaming container for flattened 2D CAD-derived drawings, whose mathematical
geometry semantics are aligned to the applicable constructs of **ISO 10303-42:2025**.

PVec is not a replacement mathematical geometry standard. Where ISO 10303-42 already defines the
geometry, PVec adopts those semantics and adds only what large web/GIS drawings need on top:
spatial indexing, HTTP range streaming, progressive visual coverage, independent compression
blocks, capability and version negotiation, CPU/WASM-friendly decoding, GPU-oriented rendering
data, optional non-geometric components, and deterministic interoperability reports.

## Status

**Phases 1 and 3 are in; the container, the streaming runtime and the interchange are not.**

The work is planned as eight phases and 64 issues under [#1](https://github.com/libviprs/pvec/issues/1).
Phase 1 was a spike: the semantic profile, the numeric policy and eight ADRs, decided after the
candidate layouts and STEP-HDF had been benchmarked on identical fixtures. Phase 3 is the geometry API,
the capability ABI and the component model, as crates:

| crate | what it is |
|---|---|
| `pvec-numeric` | the one numeric policy: tolerances, degeneracy rules, limits |
| `pvec-geometry` | the source-neutral curve model with the one exact evaluator, bounds and affine transforms |
| `pvec-capabilities` | the capability registry as data, reader negotiation and extension governance |
| `pvec-entity` | the file-local `EntityId` and the table that resolves it to bounds |
| `pvec-codec` | the geometry codecs: curves to the type-batched block payload of ADR 0004 and back |
| `pvec-components` | independent component streams (layer, line style, text, hatch, provenance) |
| `pvec-checks` | repository checks, including the public-API snapshots and the crate boundaries |

The container itself, spatial indexing and streaming, interchange and the public API freeze are later
phases. Phase 2 (the dual-oracle conformance suite) is not done, so nothing here has been measured
against the converters yet: the exactness claims are those of the tests in this repository.

## The rule that decides the design

The **semantic API and specification are normative; the physical encoding is replaceable.** No
serialization-generated type appears in the public geometry API.

Three conformance classes are kept distinct everywhere, and a display-perfect result is never
labelled mathematically exact:

| Class | Claim |
|---|---|
| Structural exactness | equivalent parameters where directly comparable — degree, control points, knots and multiplicities, weights, periodic/closed state, trim/domain |
| Geometric exactness | equivalent locus under a documented numeric policy, even where parameterisation differs |
| Display fidelity | a bounded rendering approximation under an explicit screen-space/world-space error budget |

## Related repositories

| Repository | Role |
|---|---|
| [`libviprs/pvec-gold-fixtures`](https://github.com/libviprs/pvec-gold-fixtures) | the containerised dual-converter DWG oracle and the fixture corpus this repository's conformance suite reads |
| [`libviprs/libviprs`](https://github.com/libviprs/libviprs) | DWG and vector-PDF ingestion, normalised into PVec semantic geometry |
| [`libviprs/maplibre-pvec`](https://github.com/libviprs/maplibre-pvec) | the MapLibre GL JS add-on that streams `.pvec` and renders it |

Dependency direction is one way: `pvec-gold-fixtures` → `pvec` → { `libviprs`, `maplibre-pvec` }.
