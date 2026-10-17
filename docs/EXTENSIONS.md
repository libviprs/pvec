# Extension governance

PVec adds a custom geometry type only when it has to. An unchecked custom type would quietly break the
one claim PVec makes about its mathematics, which is that it is aligned to named ISO 10303-42:2025
constructs and validated through STEP interchange. This page is how an extension earns its place. The
gate is code (`pvec-capabilities`, `Registry::register_extension`), and this page says what the code
checks and why.

## The profile rule

The profile rule, stated in `docs/ISO_PROFILE.md`, applies here before anything else:

> A distinct DWG (or PDF, or any source) entity class is not a reason for a distinct PVec mathematical type.

An `ARC`, a `CIRCLE` and an `ELLIPSE` that happens to be round already land on the types of the profile.
Source mapping belongs to EPIC N in `libviprs/libviprs`. So "the source format has an entity for it" is
never an argument in a proposal, and a proposal that rests on it is refused. An extension is justified
only when no suitable ISO 10303-42 representation exists. Style, text and hatch are not geometry
extensions at all: they are component streams (P3.7) and stay outside the geometry oracle.

## What a proposal has to carry

In short a proposal needs the Part 42 argument, a mathematical definition, bounds and evaluation rules,
tessellation rules, STEP and GIS behaviour, and fixtures that carry an evidence class. It is a markdown
file with these sections, in the shape of `docs/extensions/TEMPLATE.md`:

1. **Part 42 constructs considered.** Proof that no suitable ISO 10303-42 representation exists, naming
   each Part 42 construct considered and saying why it fails. One entry per construct, written as
   ``- `construct`: why it fails``.
2. **Mathematical definition.** The curve, its parameter, its domain.
3. **Bounds and evaluation rules.** Closed form bounds, and how a point and a derivative are computed.
4. **Tessellation rules.** How a tessellator is to receive the exact curve. Tessellation itself is P7.2.
5. **STEP behaviour.** Either an exact form, or the sentence "no exact representation; bounded
   approximation with error reported". An exact form may not be recorded as exact without a passing
   round-trip fixture: a fixture line has to say `round-trip: passing`.
6. **GIS behaviour.** The same, and where it is not exact, a line `fidelity class: <class>` naming the
   class of the fidelity taxonomy (P6.1).
7. **Conformance fixtures.** Each fixture classified under EPIC O's evidence classes, spelled exactly:
   `KNOWN_TRUTH_EXACT`, `DUAL_ORACLE_EQUIVALENT`, `REAL_WORLD_DIFFERENTIAL` or `QUARANTINED`. The
   vocabulary belongs to EPIC O; a fifth class is refused. At least one fixture must not be quarantined,
   because a quarantined fixture can neither pass nor fail a gate.

A section that is absent, empty, still holds a `TODO` or is only a word or two counts as missing, and
the checker names each missing artifact on its own, so one gap cannot hide behind another.

## The gate

`Registry::register_extension` refuses an extension, and registers nothing, unless:

* the entry is of class extension, takes the next dense capability id and a name nobody has;
* it names no ISO construct (an extension exists because none fits);
* the proposal passes the checker above;
* the STEP behaviour in the entry agrees with the proposal.

Extensions are optional capabilities by default. Marking one required fails validation unless an
accompanying ADR is listed in the registry entry (the `adr` field), because a required capability makes
every reader without it refuse the file. A file cannot declare an optional extension required either.

## Examples

`docs/extensions/TEMPLATE.md` is the empty form and does not pass the checker until it is filled in.
`docs/extensions/sample-proposal.md` is a complete proposal for a hypothetical hyperbolic arc. It is a
sample: it is not registered and it proposes nothing for v1. It exists so that the gate has an
example that passes end to end, and a test deletes each of its sections in turn and checks that the
right artifact is reported.

## What is not here

Proposing actual extensions, capability negotiation (P4.7), the fidelity taxonomy itself (P6.1) and
source-format semantics (EPIC N).
