# PVec v1 freeze record

Status: prepared for review, 2026-10-25. v1 is not tagged and nothing is released from this page. It records what v1
promises, what is ready to be frozen, and what is not, so that whoever tags it can see the state in one place. It decides
nothing that an ADR has not already decided, and it re-runs and amends no measurement.

## What is frozen, what is not

A row is `FROZEN` only when its result file exists and records no failed gate. Anything else is `NOT FROZEN`, with the
reason in the result column. A test (`crates/pvec/tests/freeze_v1.rs`) reads this table and refuses a `FROZEN` row whose
result file records a failure, so the table cannot say more than the files do.

| Frozen artefact | Decided by | Benchmark gate | Result file or reason | Status |
|---|---|---|---|---|
| semantic profile (P1.1) | ADR 0001 | P2.5 + P2.6 green | crates/pvec-harness/tests/issue_41_gates.rs | FROZEN |
| numeric policy (P1.2) | ADR 0002 | P2.6 pathological suite green | tests/pathological/policy_cases.rs | FROZEN |
| physical container | ADR 0003 | P8.2 .pvec vs Part 21 vs STEP-HDF | docs/bench/STEP_COMPARISON.md records a red gate on x64 (spline-heavy-1e6 viewport p95, 10.7x STEP-HDF), tracked in pvec#192 | NOT FROZEN |
| internal layout (SoA + arenas) | ADR 0004 | P8.3 layout arms | docs/bench/LAYOUT.md records the SoA gate red on x64 (2.01x against 3x) and the block size gate red, pvec#192 | NOT FROZEN |
| spatial index + straddler strategy | ADR 0005 | P8.3 index arms, total viewport I/O | docs/bench/LAYOUT.md records the index gate red on 7 of 7 entries (3x to 38x the bytes of the best alternative), pvec#192 | NOT FROZEN |
| progressive tiers | ADR 0007 | P8.5 coverage stages | no docs/bench/PROGRESSIVE.md: the progressive benchmark is not merged, so this gate is missing | NOT FROZEN |
| extension rules | ADR 0008 and ADR 0013 | registry lint + P7.6 matrix | tests/compat/matrix.txt | FROZEN |
| capability registry + compatibility contract | ADR 0010 | P7.8 capability matrix | docs/capabilities.toml | FROZEN |
| security and resource limits (P4.8) | ADR 0008 and docs/SECURITY_LIMITS.md | P7.8 findings all fixed | the findings are fixed, but the 4 hour nightly fuzz run is still outstanding (pvec#190), so the gate is not closed | NOT FROZEN |
| fixture corpus revision | EPIC O revision pin | corpus.lock.json digests | the chunk ordering change of pvec#159 moved the bytes of 9 of the 10 generated entries after the lock was written, so `every_digest_in_the_lock_matches_the_rebuilt_artifact` in crates/pvec-bench/tests/issue_60_corpus.rs is red on main. pvec#196 resolves it with a second lock (pvec#197): corpus.lock.json stays as the corpus the benchmark reports were measured on and cite, and corpus.post159.lock.json pins the current corpus that the corpus test checks. Not frozen until pvec#197 merges | NOT FROZEN |
| C/FFI answer | docs/adr/ADR-P7.4-c-ffi-strategy.md | prototype result in that ADR | the ADR is still Proposed; v1 ships no C header, so the C/FFI surface is deferred and nothing about it is frozen | NOT FROZEN |

## Constants

At the freeze the constants are: container 1.0 (`Header::MAJOR` 1, `Header::MINOR` 0), capability format version 1.0.0
(`FORMAT_VERSION`), and `wasm_abi_version` 1. The registry snapshot `docs/capabilities.toml` has sha256
`3d20a14b894b6268232446fe60ab68dbf2190f68287c69b74be3779eb7583887`, and the corpus lock
`crates/pvec-bench/benches/corpus/corpus.lock.json` has sha256
`9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`. The freeze tests recompute both, so changing the
registry or the corpus lock turns them red until this page is updated on purpose. The lock digest is recorded so a
regenerated lock shows up here, but the corpus is not frozen yet (see its row above)...

## The five benchmark reports

- [x] docs/bench/STEP_COMPARISON.md (P8.2): present, one gate red on x64.
- [x] docs/bench/LAYOUT.md (P8.3): present, three gates red.
- [x] docs/bench/GEOMETRY.md (P8.4): present, several gates red on both platforms.
- [ ] docs/bench/PROGRESSIVE.md (P8.5): missing.
- [ ] docs/bench/TUNING.md (P8.6): missing, so nothing has been tuned against the measurements.

The red gates are listed, with the proposed way to deal with each, in pvec#192. I have not hidden or relaxed any of them
and I have not changed a threshold to make a row green.

## What is outstanding, and not decided here

* The 4 hour nightly fuzz run is still outstanding (pvec#190). The shorter smoke runs in `docs/FUZZING.md` found nothing,
  but they are not the gate the issue names.
* The C/FFI header is deferred. v1 ships no C header, following the C/FFI ADR, which is itself still Proposed.
* The EPIC L decision (whether the analytic PVec path or L's tessellated DWG, MVT and PMTiles path is the default blueprint
  path) needs L's measured PMTiles numbers on the same drawings next to the P8.2 and P8.5 numbers. Those live in another
  repository and P8.5 is not merged, so I have not written the decision and I will not guess it. It is the owner's call.
  The reversal conditions and the cross-reference to N8.7 go in with it.

## The ISO statement

PVec v1 is aligned to named ISO 10303-42:2025 constructs, validated through ISO 10303-21 interchange, with
no claim of formal Part 42 conformance. The Unsupported Part 42 features, reproduced from `docs/ISO_PROFILE.md`:

- 1. Cartesian point: 1 and 3 coordinate forms (not 2D); `point_on_curve`, `point_on_surface`, `point_replica`, `degenerate_pcurve` (they are derived from other geometry, and PVec carries the derived coordinates instead)
- 2. Direction and vector: 3D directions; zero-magnitude vectors (Part 42 permits a magnitude of zero in some contexts, PVec does not, because a zero vector has no direction to evaluate)
- 3. Axis placement 2D: `axis1_placement`, `axis2_placement_3d` (3D only); an omitted reference direction on input is read as `(1,0)` only by the Part 21 importer, never by the container decoder
- 4. Line: a `vector` of magnitude other than 1 as stored form (an importer rescales the trim parameters; the container never stores it); 3D lines
- 5. Polyline (polyline-equivalent): bulge or arc segments inside a polyline (those are a composite of lines and trimmed circles, section 9); variable width; per-vertex data
- 6. Circle: `hyperbola` and `parabola` (the other `conic` members: no v1 source produces them, and they are unbounded, so they would need their own bounds policy; they can arrive through the extension rules of P3.5)
- 7. Ellipse: the same conic members as section 6
- 8. Trimmed curve: trims given as `cartesian_point` (the point form needs a projection step that has no single answer near self-approach); `.CARTESIAN.` and `.UNSPECIFIED.` master representation; trimmed curve over a surface curve or `pcurve`
- 9. Composite curve: `.DISCONTINUOUS.` transitions (a gap is not a curve; the producer emits separate entities); `composite_curve_on_surface`; a segment whose parent is a `pcurve`
- 10. Bezier: `quasi_uniform_curve`, `uniform_curve`, `b_spline_curve_with_knots` knot-spec forms (`.UNSPECIFIED.` is written, and the explicit knots are the truth)
- 11. B-spline with knots: the knot-spec shortcut subtypes (`uniform_curve`, `quasi_uniform_curve`, `piecewise_bezier_curve`), which carry no information beyond a particular explicit knot vector; `b_spline_curve` without knots; `rational_b_spline_curve` with non-positive weights; periodic wrap-around as a stored form (the unwrap above is a conversion, not a representation); `b_spline_curve` on a surface
- 12. Rational B-spline (NURBS-equivalent): as section 11; zero or negative weights; weights as a separate homogeneous dimension
- 13. 2D affine transform: the 3D operators and per-axis scale; `item_defined_transformation` and `representation_map` as an instancing mechanism (instancing is a PVec component concern, not a geometry one); perspective and projective maps

## What this page does not do

It does not tag v1, publish a crate or create a release, it does not design v2, and it does not re-run or amend any
measurement. `docs/MIGRATION.md` says what a v1.x reader must keep working.
