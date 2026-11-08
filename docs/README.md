# PVec documentation

This is the v1 document set. The normative statements live in the documents linked here, once each: other pages cite
them and do not restate them. Where a page needs a tolerance it cites [NUMERIC_POLICY.md](NUMERIC_POLICY.md), which is the
only page that defines one.

## Start here

* [SPECIFICATION.md](SPECIFICATION.md): what a v1 file is, assembled from the frozen artefacts, with a checklist for a third-party reader.
* [FREEZE-v1.md](FREEZE-v1.md): what is frozen and what is not yet, with the ADR and the benchmark gate of each row.
* [MIGRATION.md](MIGRATION.md): what a v1.x reader must keep working, what may change in v1.x, what needs v2.

## The format

* [CONTAINER.md](CONTAINER.md): header, manifest, blocks, writer, negotiation, and an annotated hex dump of a real file (chapter 6).
* [GEOMETRY_ENCODING.md](GEOMETRY_ENCODING.md): how geometry is encoded in blocks.
* [RASTER.md](RASTER.md): rasters inside an artifact, the placement and the index and tile blocks (ADR 0018).
* [CALIBRATION.md](CALIBRATION.md): the scale and the georeference of an artifact, the kind 11 block and its CAL rules (ADR 0019).
* [CAPABILITIES.md](CAPABILITIES.md) and [COMPATIBILITY.md](COMPATIBILITY.md): the capability registry and the compatibility rules.
* [EXTENSIONS.md](EXTENSIONS.md): how an extension is proposed and accepted.
* [ISO_PROFILE.md](ISO_PROFILE.md): the ISO 10303-42 constructs PVec is aligned to, and the ones it does not support.
* [NUMERIC_POLICY.md](NUMERIC_POLICY.md): the numeric robustness policy (P1.2).
* [STREAMING.md](STREAMING.md): the spatial directory, straddlers, Tier 0 and viewport plans.
* [SECURITY_LIMITS.md](SECURITY_LIMITS.md): the resource ceilings a reader enforces.
* [FIDELITY_TAXONOMY.md](FIDELITY_TAXONOMY.md) and [INTERCHANGE.md](INTERCHANGE.md): fidelity classes and the interchange mappings.

## Using it

* [API.md](API.md): the Rust facade. [WASM_ABI.md](WASM_ABI.md): the packed buffers of the browser build. [READER.md](READER.md): the container reader.
* [CONFORMANCE.md](CONFORMANCE.md): the gates and the three verdicts.
* [BENCHMARKS.md](BENCHMARKS.md): every phase 8 threshold with its declared limit, measured number, corpus digest and platform.
* [FUZZING.md](FUZZING.md): the hardening report.

## Checks that keep these pages true

`cargo test -p pvec-checks --test issue_67_docs` fails on a missing document, a document not linked here, a broken internal
link, a tolerance defined outside the numeric policy, or a claim of formal ISO conformance. `tools/docs-check.sh` adds
rustdoc with `-D warnings` and the doctests.
