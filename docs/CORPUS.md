# The known-truth fixture corpus

This is how `libviprs/pvec` consumes the synthetic fixtures of EPIC O (issue P2.1). The loader is
`crates/pvec-corpus`, the data is under `tests/corpus/`, and the tests are
`crates/pvec-corpus/tests/issue_34_*.rs`.

## What is pinned

| | |
|---|---|
| source | `libviprs/pvec-gold-fixtures`, commit `1df9d1b3e75047fa3978ebd809d0128cdfb28a68` (merged PR #95; the tag `corpus-r0-open` still names the older `f94f7f22`, so the commit is the pin) |
| revision | `r0-open`, status `CANDIDATE` |
| bundle | `tests/corpus/r0-open.tar`, a deterministic tar of `corpus/r0-open/` |
| bundle sha256 | `b0e3377be9581fac4c0e172982bdeb3319f93c95383428823097ef82918c3eeb` |
| manifest sha256 | `aaa7fcac8eeecd5e0e60dd7084ca40601df9194967107cd0330ab3f515b98ace` |
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

`tools/corpus-vendor.sh` is the maintenance path. It checks that the pinned commit is in the checkout,
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
- A scalar may add `"precision": N` (1 to 17), the significant decimal digits it declares (issue #95). Digits are
  counted as written: leading zeros never count, trailing zeros count after a decimal point (`1.50` is three,
  `1500` is two). A decimal with more digits than declared fails the load, and so does a declaration below the
  digits the hex needs to round-trip. `r0-open` declares none, which is accepted; the check starts to bite when a
  vendor revision carries declarations.
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
capability named. `layer-state-001` exercises no geometry capability, and the manifest says so: its layer state
travels in the layer component stream (2.0, ADR 0014 amendment), which the generator writes.

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

## Containers: `pvec-hermetic`

`pvec-corpus` stays process-free on purpose. A corpus revision that needs a converter or oracle container reaches it
through `crates/pvec-hermetic`, which is the one crate here that starts a process (the `docker` client). `r0-open`
needs none, so nothing uses it yet; it exists so the rules are written and tested before the first container.

- `Container::start` refuses an image that is not pinned `name@sha256:<64 hex>`, then runs it with no network, a
  read-only root, the corpus mounted read-only at `/corpus`, no `-e`, no socket and no privilege. The docker client
  itself sees `PATH` and the daemon-selection variables (`DOCKER_HOST`, `DOCKER_CONFIG`, `DOCKER_CONTEXT`) and nothing
  else, so no token is forwarded. No image and no credential lives in this repository.
- The container is removed by `stop()` or by `Drop`, so a pass, an early return and a panic all clean up. A start that
  failed half way is removed by name once.
- Docker missing or its daemon not answering can be skipped only when `PVEC_DEV_RUN` is `1` or `true`, and a `CI`
  that is set (anything but empty, `0` or `false`) overrides it, so CI fails. With neither variable the run is strict
  and fails. A container Docker refused to start, or an unpinned image, is never skipped, in any kind of run.
- `exec_stream` starts a command in the running container with its standard input and output piped to the test, for a
  helper the test talks to over one `docker exec -i` (the load-shed range source of `pvec-stream`, pvec#158). The
  container keeps `--network none`, so there is no port to publish and the channel works from a toolchain container too.
- The tests drive a fake `docker` found through an explicit search path, so they need no Docker and do not touch the
  process environment.

## The dual STEP oracle adapter (issue P2.2)

`pvec_corpus::oracle` loads the converter artifacts EPIC O produces for a fixture. It never runs a converter
(both SDKs are x86_64-only and not redistributable); the files arrive with their digests. A fixture's directory
holds `oracle.json` (schema `pvec/oracle-artifacts/1`), `primary.step` (CAD Exchanger), an optional
`secondary.step` (ODA), the evaluator record each went through (`*.verification.json`) and the
implementation-neutral `signature.json`.

- `oracle.json` records per artifact the converter, the SDK name and version, the host class (`x86_64-native`),
  the machine and the driver (`tools/nas.sh`). A missing or different field fails the load, and so does a file
  that does not match its digest.
- Both STEP outputs go through the same `step_eval`. The evaluator name, version, kernel version and numeric
  policy have to be identical on both paths, otherwise `EvaluatorMismatch`. The adapter keeps the STEP digests
  and drops the bytes, so there is nothing to compare one converter with the other directly.
- A fixture with one STEP output carries no dual-oracle evidence: `dual()` returns `DualOracleUnavailable`
  naming `REAL_WORLD_DIFFERENTIAL` or `QUARANTINED`, and a manifest that claims `DUAL_ORACLE_EQUIVALENT` with
  one output is refused at load. The class itself is read from the manifest and never changed here.
- `report_line` describes converter agreement as agreement, and `check_report_wording` is a ban list over the
  phrases that would turn it into a statement about the source.
- `tests/issue_35_licence.rs` scans the repository for a converter image in a build or CI file and for credential
  patterns, so the licensing gate is asserted.

The signature layout (`pvec-gold/signature/*` with a `fixture_id`) is what the adapter requires today; EPIC O's
O2.4 owns the rest of it.
