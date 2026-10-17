# ADR 0009: The pvec-geometry public model

Status: Accepted, 2026-10-16. Decides the shape of P3.1 (`crates/pvec-geometry`). Evidence:
`docs/spikes/01-3-geometry-api.md` for the prototype and the tests in `crates/pvec-geometry/tests/`
for the production crate.

## Context

The P1.3 prototype proved the trait shape and left three questions open for the production crate: how
the control points and knots are owned, whether the trait is open, and how a violation is reported. A
fourth came with the issue text: how the crate keeps serialization and wire types out of its public
API for good, and not only on the day somebody looked.

## Options

| question | options | evidence |
|---|---|---|
| storage of control points and knots | A. generic over owned or borrowed storage (the prototype). B. owned `Vec` only | the prototype measured evaluation within 2.4% either way and build cost dominated by validation (202 ns against 181 ns at 8 control points), so the copy B makes does not show. A puts `<P, F>` on every spline type and the closed `Curve` enum can only hold the owned instance anyway |
| the trait | A. open, `Box<dyn Curve2d>` allowed. B. sealed, with a closed enum over the nine kinds | the prototype measured 29.2 ns for the enum and 30.3 ns for a trait object, a 4% gap I do not rest anything on. The profile fixes the set of kinds and a decoder has to reject anything else, which only B guarantees |
| errors | A. one flat enum. B. wrap the numeric policy's error and add the few geometry-level refusals, each naming a rule | a rule id (`NP-nn` or `PROFILE-n`) on every error lets a test, a log line and a hostile-input report point at the text that was violated |
| keeping wire types out | A. a review habit. B. a dependency allow-list, a source scan and a committed public API snapshot, all in `pvec-checks` | B fails the gate by itself; the dependency allow-list is what makes `wasm32` and the downstream dependency policy stay true |

## Decision

B in every row.

* Owned storage. A reader that wants zero-copy access reads the block bytes itself (ADR 0004) and builds
  a curve with one validated copy per curve. If P7.8 shows that copy mattering, the reversal below applies.
* `Curve2d` is sealed and `Curve` is a closed enum marked `#[non_exhaustive]`, so a new kind (arriving only
  through P3.5) is not a breaking change for a downstream `match` that already has a wildcard arm.
* `GeometryError::Numeric(NumericError)` plus `OutsideDomain`, `Unbounded`, `BadBasis`, `BadSegment` and
  `WeightCount`, every one with `rule()`. A successful call never returns a non-finite number: an overflow
  in an intermediate is refused as `NP-02`.
* A derivative of order zero is the point, as a vector; derivatives are evaluated in place on the
  `degree + 1` control values around the span, which removes the per-call allocation the prototype had.
* `Trimmed::from_normalised` exists next to `Trimmed::new`, so a decoder can rebuild a trim bit for bit
  from `Trim { start, span, reversed }` without re-normalising (NP-11 would otherwise move the last bit).
* The closed flag of a Bezier, B-spline or composite is a claim that is checked when declared (NP-20) and
  never inferred.
* Bounds are closed form for lines (once trimmed), arcs, ellipses, polylines and Bezier curves up to degree
  3, and the control hull above that and for splines. The hull is an enclosure, said so in the type's docs;
  tight spline bounds need root finding per span and are not claimed.
* `crates/pvec-checks` holds the boundary: an allow-list of dependencies per API crate, a scan that bans
  file, network, process and environment access and `unsafe`, and a textual snapshot of the public API
  (`crates/<crate>/api.txt`) that has to be re-blessed on purpose when the surface changes. The snapshot is
  text-based, not type-resolved, because a type-resolved one needs a nightly rustdoc; the banned-name scan runs
  over it and over the manifest.
* The pathological suite moved into `pvec-geometry`'s test targets and its 15 evaluator probes run in the
  ordinary test run. The gate no longer expects them to fail.

## Consequences

* Every other crate in phase 3 depends on `pvec-geometry` and never the other way round, which keeps a
  generated wire type from ever being nameable in the geometry API.
* The `Result` alias is not exported, so `use pvec_geometry::*` cannot shadow the prelude.
* Whether a trait impl or a public field changed is visible as a diff of `api.txt`, so API review does not
  depend on anyone remembering to look.
* Fifteen hand-written mutations of the evaluator (a sign, a factor, a skipped validation) each turn at least
  one test red. That is a spot check of the suite, not a coverage claim.

## Reversal threshold

Switch the reader-facing types to borrowed storage if P7.8 or P8.4 shows that the per-curve copy costs more
than 10% of decode time on the spline-heavy corpus. Open the trait only if a use case appears that the
extension rules of P3.5 cannot serve. Replace the textual snapshot with a rustdoc JSON one when the toolchain
policy allows a nightly in the gate.
