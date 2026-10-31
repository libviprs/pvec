# Conformance gates

Status: frozen by issue P2.8 (`libviprs/pvec#41`). This page says what each verdict requires, so that a
claim in one of the four repositories means the same thing in all of them. It exists because panel finding 8
asked for mathematical, structural and display results to be kept apart, and I would rather that be
mechanical than editorial.

The runner is `crates/pvec-harness` (`gates.rs`), the report writer and the reader an older consumer
uses are in `report.rs`, and the tests are `crates/pvec-harness/tests/issue_41_gates.rs`.

## What I claim

The one claim PVec makes about the standard is the sentence in [`ISO_PROFILE.md`](ISO_PROFILE.md) (see
`docs/ISO_PROFILE.md`), and this page repeats only that:

> PVec is aligned to named ISO 10303-42:2025 constructs and validated through STEP interchange.

None of the five gates below is a statement about the standard itself. They are statements about PVec's
own parameters, loci and files, and each one says which kind of evidence can back it.

## Where the numbers come from

This page introduces no tolerance. Every threshold a gate uses is a rule of
[`NUMERIC_POLICY.md`](NUMERIC_POLICY.md) (see `docs/NUMERIC_POLICY.md`), reached through `pvec_numeric::Policy`
by the structural module (P2.5) and the geometric module (P2.6). The display gate takes its budget as an
input and records it beside the error; it does not decide what a good budget is. A repository check keeps a
number written as a tolerance out of the gate modules.

## The five gates

The evidence classes are EPIC O's four and are only cited here: the class of a fixture is read from O's
manifest and the runner never changes it. A quarantined fixture runs, but it cannot satisfy any gate.

| Gate | Requires | Evidence classes | Required evidence | Required tests | Required report fields |
|---|---|---|---|---|---|
| structural exact | every field of P2.5 equal under the equivalence precondition | KNOWN_TRUTH_EXACT, DUAL_ORACLE_EQUIVALENT | the fixture's reference parameters, read from the manifest or the independent evaluator | P2.5 structural suite | gate, evidence class, degree, knot count, control point count, failing fields |
| geometric exact | deviation within the policy tolerance across the whole sampling strategy of P2.6; the parameterisation may differ | KNOWN_TRUTH_EXACT, DUAL_ORACLE_EQUIVALENT | the independent STEP evaluation (O1.3) as the reference side | P2.6 geometric suite | gate, evidence class, maximum deviation, worst parameter, tolerance, arc length deviation |
| display approximate | bounded screen-space and world-space error, with the budget stated; never labelled exact | KNOWN_TRUTH_EXACT, DUAL_ORACLE_EQUIVALENT, REAL_WORLD_DIFFERENTIAL | the tessellation and the budget it was held to | the display tests of phase 5 | gate, evidence class, screen error, screen budget, world error, world budget |
| format valid | header, directory, integrity and limits accepted by the safe reader | KNOWN_TRUTH_EXACT, DUAL_ORACLE_EQUIVALENT, REAL_WORLD_DIFFERENTIAL | the container and the reader's verdict | P2.7 hostile suite green | gate, evidence class, cases run |
| forward compatible | unknown optional blocks skipped without parsing their payload; capability negotiation per P3.4 | KNOWN_TRUTH_EXACT, DUAL_ORACLE_EQUIVALENT, REAL_WORLD_DIFFERENTIAL | a container with an optional block the reader does not know, among them `tests/compat/raster-degraded.pvec`, whose kind 9 and kind 10 entries carry capability 21 (the payload is opaque at this stage: the gate checks that the entries are skipped and the geometry is bit-identical, not their content) | P3.4 negotiation suite | gate, evidence class, blocks skipped |

A real-world differential fixture can show that nothing moved from a recorded baseline. It cannot show that
a source parameter survived, so it cannot satisfy either exact gate, and the runner refuses the claim: the
test `issue_41_the_runner_refuses_a_gate_the_class_cannot_satisfy` is the proof. The same goes for
reports built by P2.5 and P2.6 for such a fixture, which carry the words "regression stability" and go into
the baseline report instead.

## The gates are independent

A fixture can pass the geometric gate and fail the structural one, and the runner reports both rows. There
is no overall tick, and no function that merges the gates. A structural comparison whose equivalence
precondition fails (a reparameterised spline, a reversed curve) is reported as `ROUTED` to the geometric
gate: it is neither a pass nor a failure of the structural one.

## The three verdicts

The five gates above give three kinds of verdict, and each has its own procedure and none is derived from another. **structural exactness** is the structural exact gate: the stored parameters equal the reference field by field. **geometric exactness** is the geometric exact gate: the curve deviates from the reference by no more than the policy tolerance, wherever it is sampled. **display fidelity** is the display approximate gate: a stated screen-space and world-space error budget was met, and the verdict says approximate in its own wording. The runner is the `pvec-conformance` procedure of this page (`crates/pvec-harness`, the P2.8 gates), and it never merges the verdicts. They are kept apart because they answer different questions, so a display-perfect result is not mathematically exact, and no page of this documentation may word one as the other.

## Wording

The formatter has one word per gate and verdict (`Gate::word`). The display gate has no word for exactness
(`approximate, within budget`, `approximate, over budget`), so a display result cannot be rendered as exact,
and a test enumerates every verdict to show it. A geometric pass is never worded as structural exactness,
and display fidelity is not asserted by the geometric gate at all. Text that describes converter agreement
as source truth is refused by the report check from P2.3.

## The report

The runner writes `pvec/conformance-report/1`, a JSON document with a `schema_version`, and a `rows` array.
A row has `gate`, `fixture`, `evidence_class`, `verdict` (`PASS`, `FAIL` or `ROUTED`) and `numbers`, a list of
named values. Every real is a `{decimal, f64_hex}` scalar like in the corpus, and a value that is not finite
is written as a `non_finite` marker, so a deviation of infinity survives the trip.

Inside a major version a field may be added to the document or to a row. A consumer of version 1 ignores what
it does not know, and a test adds a field in both places to prove it. A new major version changes the schema
string, and a version 1 consumer refuses it instead of half-reading it.

## Not decided here

Thresholds tuned after the benchmarks (P8.6, which may not weaken these gates), the interchange fidelity
taxonomy (P6.1), the v1 freeze itself (P8.7), and the reader that satisfies the format gate (P4.5).
