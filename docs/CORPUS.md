# The known-truth fixture corpus

This is how `libviprs/pvec` consumes the synthetic fixtures of EPIC O (issue P2.1). The loader is
`crates/pvec-corpus`, the data is under `tests/corpus/`, and the tests are
`crates/pvec-corpus/tests/issue_34_*.rs`.

## What is pinned

| | |
|---|---|
| source | `libviprs/pvec-gold-fixtures`, tag `corpus-r0-open` (commit `f94f7f22b7aaff36eaaf6d5501fafc42054f22f2`, merged PR #43) |
| revision | `r0-open`, status `CANDIDATE` |
| bundle | `tests/corpus/r0-open.tar`, a deterministic tar of `corpus/r0-open/` |
| bundle sha256 | `901370036bbffbbb8cb30471c2bf90f171b4f966c5bdfea6b140591cd840ca50` |
| manifest sha256 | `de3903df3e4d7bc0556a2d593298aaea34dcdbdcdec9015d753df14f1566bf21` |
| pin file | `tests/corpus/r0-open.pin.json`, byte for byte the file the tag publishes |
| numeric policy | `PVec numeric policy v1` |

The bundle is 1.5 MB (30 DWGs, their read records and the authored `expected.math.json` files) and every
DWG is marked `PUBLIC_REDISTRIBUTABLE` in the manifest.

## Decision: vendored, not fetched

The bundle is committed to this repository and checked against the pin on every load. I weighed three ways
to get it into a test run:

1. Fetch at test time. Rejected: it needs the network, and a hermetic suite cannot.
2. Fetch in a container step with the hash verified, then mount it. It works, but it adds a Docker
   dependency and a moving part to a corpus that is 1.5 MB.
3. Vendor it. Chosen. The suite reads two files from the checkout, the SHA-256, tar and JSON readers are in
   the crate, and nothing opens a socket or starts a process (a test scans the sources to keep it that way).

`tools/corpus-vendor.sh` is the maintenance path. It checks that the tag still resolves to the pinned commit,
rebuilds the bundle with the corpus's own bundler in a throw-away container, and compares the result with
what is vendored. Moving to a new revision is: change the constants in that script, run it with `WRITE=1`,
and update the digests that the tests spell out (they are written out on purpose, so replacing the tar and
the pin file together does not go unnoticed).

## What a load checks

In this order, and the first failure wins:

1. the tar parses (regular files only, sizes and names bounded, checksums verified);
2. the manifest's `corpus_revision` equals the pinned revision, otherwise the error names both;
3. the tar's SHA-256 equals the pin, the manifest's SHA-256 equals the pin, and every file equals its line in
   `SHA256SUMS`;
4. the numeric policy named by the manifest is the one the pin names and the one the comparison code implements.

Reading an `expected.math.json`:

- JSON is read strictly. A number with a fraction or an exponent is refused, so a bare float anywhere in a
  record fails the load. Duplicate keys are refused.
- Every real is a `{decimal, f64_hex}` scalar. The hex is authoritative (`0x` and 16 lowercase digits) and the
  decimal has to parse, under round to nearest, to exactly those bits. A value routed through a 15 digit decimal
  fails the load and names the place it came from. The corpus has 886 scalars; the tests count how many of them
  need more than 15 digits, so a corpus of round numbers could not pass this check by accident.
- Non-finite values are refused (NP-01).
- The entity class has to be one of the nine curve classes of the record and carry the geometry fields its class
  needs.

I did not copy the fixture repository's exact-rational checks (determinants, composed transforms, knot counts).
Those run in that repository at the tag. What this loader adds is what it needs to feed a comparison: bit
exactness and structure.

## Evidence classes

The registry (`FixtureRegistry`) refuses a fixture with no `evidence_class`, or one outside the four of
`pvec_capabilities::EvidenceClass`, and the error names the fixture. Beyond spelling, the class has to be
supported by what the manifest records:

- `DUAL_ORACLE_EQUIVALENT` and `REAL_WORLD_DIFFERENTIAL` with `oracles: "none"` are refused.
- The manifest's class has to equal the class in the fixture's own authored record.
- `Fixture::supports(Claim)` says what a fixture may be cited for. In r0-open every fixture is
  `KNOWN_TRUTH_EXACT` with `evidence_scope: READER_MAPPING_ONLY` and `oracles: none`, so each one supports
  `Claim::ReaderMapping` and nothing else. `StepExactness` and `DualOracleAgreement` are false for all of them.
- The three probes the corpus could not write (`general-affine-001`, `shear-001`, `singular-001`) are registered
  as `QUARANTINED` with no file and no record, and support nothing. `near-singular-001` is accepted (its DWG
  carries the transform as authored) but carries a downstream disposition, so `must_not_emit_curve` is set.

The class paths for `DUAL_ORACLE_EQUIVALENT` and `REAL_WORLD_DIFFERENTIAL` are exercised only by doctored
manifests in the tests. No fixture of r0-open is in either class.

## Capability names

The corpus spells some capabilities differently from the registry in `crates/pvec-capabilities`:

| corpus | registry |
|---|---|
| `bezier` | `bezier_curve` |
| `bspline` | `b_spline_curve` |
| `nurbs` | `rational_b_spline_curve` |
| `affine_transform` | `affine_transform_2d` |
| `line`, `polyline`, `circle`, `ellipse`, `trimmed_curve`, `composite_curve` | the same |

That table (`CORPUS_NAMES` in `registry.rs`) is the only translation, a registry name is also accepted as
itself, and a name in neither column fails registration naming the fixture and the capability. A geometry
capability also needs its numbered section in `docs/ISO_PROFILE.md`; without one the fixture fails with the
capability named. `layer-state-001` exercises no capability: the 2D profile has none for layer state, and the
manifest says so.

Two things the corpus and the profile do not line up on, which matter to the tasks this feeds (P2.3 to P2.8):

- The authored records are three dimensional (`center`, `normal`, a 4x4 transform matrix), the PVec profile is
  two dimensional (NP-18). Mapping a record onto the 2D profile is a decision for those tasks, not something
  the loader does.
- The affine fixtures name `affine_transform_2d`, but their records carry a 4x4 matrix.

## Comparison

`compare(Mode, expected, actual, scale)` is the P1.2 rule and calls `pvec_numeric::Policy::V1`:

- `Exact`: equal bits, with `-0.0` equal to `+0.0` (NP-03). The mode of every field the DWG stores verbatim.
- `Policy`: NP-05 `near`, with `scale` the size of the feature (NP-04). The mode of every derived field.

Each fixture ships a `comparison.json` with the reader results and the verdicts the fixture repository reached
(2072 checks over both readers, 1936 `EXACT` and 136 `POLICY`). The suite recomputes every one of them with
this code and requires the same answer. The `POLICY` fields are replayed with scale 0, the strictest reading of
NP-04, so a pass holds for any feature size. The same test ties the verbatim fields back to the bits in
`expected.math.json`.

## What this does not prove

r0-open has no STEP oracle and no converter. `KNOWN_TRUTH_EXACT` here means two open DWG readers (acadrust and
ACadSharp) decode the files to the authored mathematics. The readers share a DWG specification, so a shared
misreading would not show. Nothing in this corpus says how CAD Exchanger, ODA or a STEP path treats these
drawings, and nothing in this repository may cite it for that. The dual-oracle corpus is the vendor revision
tracked in `libviprs/pvec-gold-fixtures#42`, consumed by P2.2.
