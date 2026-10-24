# Translation fidelity taxonomy

This is the only place the words `Exact`, `ExactReparameterized`, `EquivalentCompound`,
`BoundedApproximation` and `Unsupported` are defined. Every export, every interchange report and
every downstream consumer (`libviprs/libviprs#1039`, `libviprs/maplibre-pvec#1`) cites these values
and does not invent a sixth, a softer synonym or a private scale. The values are implemented in the
`pvec-interchange` crate (`src/fidelity.rs`); the crate and this document have to agree, and the
tests in `crates/pvec-interchange/tests/fidelity.rs` fail when this document stops naming a value.

The document sits beside `docs/CONFORMANCE.md` (the P2.8 gates for what a PVec file may claim about
itself), not inside it. That one is about a stored drawing; this one is about what happened to the
geometry when it left PVec for another format. Panel finding 8 (separate mathematical, structural
and display conformance) is why it is a deliverable of its own.

## What a record says

An export writes one `FidelityRecord` per entity (more than one when an entity maps to several
constructs) and accumulates them in a `FidelityLedger`:

| Field | Meaning |
|---|---|
| `entity_id` | The file-local `EntityId` of the source entity (`docs/ENTITY_ID.md`). |
| `source_capability` | The id of the capability the source geometry uses, from the registry in `docs/CAPABILITIES.md`. Never zero. |
| `target_construct` | The construct the export wrote, by its name in the target format, and whether it is `Native` or `Tessellated`. Never blank. |
| `fidelity` | One of the five values below. |
| `error_bound` | Present exactly when the value is `BoundedApproximation`: a finite, strictly positive `f64`, in drawing units. |
| `note` | Free text for a person. Never interpreted by a tool. |

`Unsupported` carries a reason (a non-blank string) in place of a bound.

## The five values at a glance

| Value | Structural exactness | Geometric exactness | Display fidelity |
|---|---|---|---|
| `Exact` | claimed | claimed | not a claim |
| `ExactReparameterized` | claimed | claimed | not a claim |
| `EquivalentCompound` | not claimed | claimed | not a claim |
| `BoundedApproximation` | not claimed | not claimed | claimed, within the bound |
| `Unsupported` | not claimed | not claimed | not claimed |

The three columns are the epic's conformance classes (`pvec#1`): structural exactness is equivalent
parameters where directly comparable (degree, control points, knot values and multiplicities,
weights, periodic or closed state, trim domain); geometric exactness is an equivalent locus under
the numeric policy, even where the parameterisation differs; display fidelity is a bounded rendering
approximation under an explicit error budget. A display-perfect result may never be labelled
mathematically exact, and the table is why: only the two exact values and the compound value claim a
geometric class, and a bounded approximation claims only the display class.

Throughout, "equal" and "the same locus" are decided under `docs/NUMERIC_POLICY.md`: points and
parameters are compared with the tolerance triple of NP-04 and the `near` of NP-05, with the scale
term the size of the feature. A mapping never gets to pick a looser tolerance and still say `Exact`.

## Exact

**Claim:** the target construct is the source construct. Same type, same parameters, the same
structure, and nothing was rewritten on the way.

**Must be true:** every structural parameter listed above is carried over bit for bit, or differs
only by `-0.0` against `+0.0` (NP-03). The target construct is `Native`. The target format can
represent the construct without a default, a unit change or a rounding step that touches a value.

**Evidence:** a round trip of the exported file through the target format's own reader back to PVec
gives parameters equal bit for bit, for the mapping's known-truth fixtures; and the ledger record
names the exact target construct. A claim made from a visual comparison is not evidence.

## ExactReparameterized

**Claim:** the same construct type and the same locus, with parameters related to the source by a
documented exact map.

**Must be true:** the map is written down for this mapping (for example a periodic start angle
shifted by a whole turn under NP-11, a knot vector affinely rescaled to a unit domain, a reversed
direction with the trim swapped, a line's origin moved along itself). The locus agrees within the
NP-04 tolerance triple of the source feature's scale when sampled at the documented points, and the
orientation and the trim end points agree. The target construct is `Native`. The map is exact in
the sense that it is a closed-form transformation, and the residual it leaves is floating point
noise inside the policy, not an approximation error.

**Evidence:** for each mapping, the map itself, plus a measured maximum deviation over the
known-truth fixtures that is inside the NP-04 tolerance. The measured number is recorded with the
mapping, not only the verdict.

## EquivalentCompound

**Claim:** the same geometric locus, written as a different structure. Geometrically exact,
structurally different.

**Must be true:** the compound reproduces the locus within the NP-04 tolerance triple, end points
and orientation included, using only constructs of the target format that are themselves exact
(for example a conic written as a chain of circular arcs and lines when that is exact, or one NURBS
span split into two at a knot). The target construct is `Native`, and the pieces join with the
continuity the source had. Sampling a curve into a polyline is not an equivalent compound.

**Evidence:** the decomposition rule for the mapping and a measured maximum deviation over the
known-truth fixtures, inside the NP-04 tolerance, including the joins.

## BoundedApproximation

**Claim:** display fidelity only. The target stands in for the geometry and no mathematical class is
claimed.

**Must be true:** a finite, strictly positive `error_bound`, the largest distance from any point of
the target to the source locus (and from any point of the source locus to the target) in drawing
units, as a proven or measured upper bound and not a typical value. The type system enforces the
shape of this: `Fidelity::BoundedApproximation` takes an `ErrorBound`, which cannot be made from
zero, a negative number, NaN or an infinity. A `Tessellated` target can only ever be recorded as
`BoundedApproximation` or `Unsupported`; recording it as any exact value is refused when the record
is built and again when a serialized ledger is read.

**Evidence:** how the bound was obtained (a chord-error formula for the tessellation parameters, or
a measured maximum over a stated sampling) and the fixture or corpus it was checked against. The
bound is stored as the exact `f64` bit pattern, so a report from another build says what this build
said.

## Unsupported

**Claim:** nothing. The entity was not translated, or was translated in a way the export cannot
describe by any of the other four values.

**Must be true:** a reason, a non-blank string saying what the target cannot represent or what the
export refused. The ledger still records the entity, so a missing entity is never silent.

**Evidence:** the reason, naming the capability and the target limitation. `Unsupported` is never a
default for "the mapping has not been analysed".

## Choosing a value

Take the strongest value whose "must be true" holds, in this order: `Exact`, `ExactReparameterized`,
`EquivalentCompound`, `BoundedApproximation`, `Unsupported`. An export that cannot show the evidence
for a value does not claim it and falls to the next one. A value is claimed per entity, so a file
whose entities are mostly exact and partly approximated reports both, and the ledger counts them
separately instead of averaging them into one grade.

## The serialized form

A ledger serializes to a line-oriented UTF-8 text format. It is written by hand in the crate (no
serialization library) and is part of the interface:

```text
pvec-fidelity-ledger 1.0
record
entity 7
capability 6
target Tessellated POLYLINE
fidelity BoundedApproximation
error_bound 0x3fe0000000000000
note 64 segments
end
ledger-end 1
```

- The first line is the magic and `MAJOR.MINOR`. A reader accepts its own major version with any
  minor, and refuses another major.
- Inside a record each line is `key value`, the key never containing a space. `error_bound` is the
  `f64` bit pattern as sixteen hex digits, which is what makes a round trip bit exact. `reason`
  appears for `Unsupported`. Text values escape `\` as `\\`, line feed as `\n` and carriage return
  as `\r`.
- A later minor version may add lines. A reader skips keys it does not know, inside a record and
  between records, so a report written by a later build is still readable by an earlier tool.
  Adding a sixth fidelity value would change what a report means, so it is a major version bump.
- `ledger-end N` is the last line and `N` is the record count, so a truncated or spliced report is
  refused rather than read short. The file ends with a line feed.
- Reading goes through the same constructors as writing. A record without a capability or a target,
  a bound that is not finite and positive, a bound on a value that does not take one, a missing
  reason, or a tessellated target claiming exactness, fails to read.

## Reading the numbers

`error_bound` is in drawing units, the same units as the source coordinates, so a consumer compares
it against its own display budget in those units. It says nothing about the structural or geometric
classes, which a `BoundedApproximation` does not claim.
