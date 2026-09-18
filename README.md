# pvec

`.pvec` is a spatial streaming container for flattened 2D CAD-derived drawings, whose mathematical
geometry semantics are aligned to the applicable constructs of **ISO 10303-42:2025**.

PVec is not a replacement mathematical geometry standard. Where ISO 10303-42 already defines the
geometry, PVec adopts those semantics and adds only what large web/GIS drawings need on top:
spatial indexing, HTTP range streaming, progressive visual coverage, independent compression
blocks, capability and version negotiation, CPU/WASM-friendly decoding, GPU-oriented rendering
data, optional non-geometric components, and deterministic interoperability reports.

## Status

**Specification stage. There is no implementation in this repository yet.**

The work is planned as eight phases and 64 issues under [#1](https://github.com/libviprs/pvec/issues/1).
Phase 1 is deliberately a spike: no binary layout is frozen until ISO/TS 10303-26 STEP-HDF and the
candidate custom layouts have been benchmarked on identical fixtures, and until the numerical
policy is approved.

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
