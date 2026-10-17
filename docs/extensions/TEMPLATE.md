# Extension proposal: TODO name

Copy this file, replace every `TODO`, and keep the section headings exactly as they are. The checker in
`pvec-capabilities` reads the headings. See `docs/EXTENSIONS.md` for what each section has to contain.

## Part 42 constructs considered

TODO: one entry per construct, written as a bullet with the construct in backticks, a colon and the reason
it fails, for example ``- `hyperbola`: why it cannot represent this curve``. Naming no construct is not an
argument.

## Mathematical definition

TODO: the curve, its parameter and its domain.

## Bounds and evaluation rules

TODO: closed form bounds, and how a point and a derivative are computed.

## Tessellation rules

TODO: how a tessellator receives the exact curve and chooses its subdivision.

## STEP behaviour

TODO: either an exact form, which needs a fixture below that says `round-trip: passing`, or the sentence
"no exact representation; bounded approximation with error reported".

## GIS behaviour

TODO: exact, or densified, and then a line `fidelity class: <class>`.

## Conformance fixtures

TODO: one bullet per fixture, `- name: CLASS`, where CLASS is KNOWN_TRUTH_EXACT, DUAL_ORACLE_EQUIVALENT,
REAL_WORLD_DIFFERENTIAL or QUARANTINED. Add `; round-trip: passing` when a STEP round trip has passed.
