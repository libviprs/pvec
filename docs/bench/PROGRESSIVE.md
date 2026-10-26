# Progressive coverage benchmark (P8.5)

This measures how long a viewer waits before it can show something true, and how much of the drawing is on screen at each stage:
latency to coverage, not a frame rate. It measures and reports only. Nothing here was tuned, no threshold was moved after the run,
and every missed threshold is a result below. P8.6 tunes against these numbers and P8.7 decides what the freeze does about each one.
The harness is `crates/pvec-bench/benches/progressive.rs`, the driver is `tools/progressive-bench.sh`, the rows are
`docs/bench/progressive.csv` (one number per row, columns `table,entry,platform,arm,param,metric,value,note`), and the test that holds
this document to that file is `crates/pvec-bench/tests/issue_64_progressive.rs`.

## Inputs and platforms

* Corpus: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`
  (the same lock the other phase 8 reports use). Entries: `pathological-1e3`, `architectural-1e3`, `architectural-1e5`, `mep-1e5`,
  `industrial-1e5`, the mandatory `huge-coordinate-1e5` and `spline-heavy-1e5`, `architectural-1e6` and the mandatory
  `spline-heavy-1e6`. Each is written once with a Tier 0 block per leaf and then read back through the streaming job with the
  200 recorded viewports of the lock.
* **native x64** is MARS (`linux/amd64` container, rustc 1.99.1, release). **arm64** is mercury (Apple silicon, the Colima VM,
  `linux/arm64`, same compiler). Every row names its platform and no verdict reads across the two. Both hosts were shared and
  lightly loaded (load averages 1 to 2.1 at the end of each entry).
* Each stage time is the p95 over the 200 viewports, and the figure in the row is the median of three repeat runs.

## What is modelled and what is measured

The link is **modelled**, the CPU is **measured**. Each range request costs one round trip before its first byte, up to six requests
are in flight, and the bytes of all requests move over one shared link in issue order (so overlapping does not buy bandwidth). The
arms are `local` (no network time at all), `25mbit-60ms` and `5mbit-200ms`. A stage ends at the modelled completion of its last
request plus the measured decode and index time on that host. Nothing was fetched over a real network.

* **Stages.** 1 is the directory (header, directory and index for the viewport). 2 is complete Tier 0 for the viewport. 3 is the
  first exact refinement, 4 is full refinement of the visible set.
* **Metric.** coverage = entities represented in viewport / entities intersecting it, where a Tier 0 preview counts as represented.
  It is 1 for an empty viewport. A viewport that loses an entity is a failure on any entry and any arm, not a slow result.
* **Previews are display fidelity, not geometric exactness.** A Tier 0 preview is a drawing aid with a stated error bound; it is not
  geometric exactness and never replaces the exact entity. `worst_preview_px` is the preview error bound times the viewport scale on a
  1920 px screen, over the viewports that show Tier 0. The writer is given a 0.5 px budget at the fit zoom (the `fit` rows), and the
  threshold is held against the worst recorded viewport. `previews_without_identity` counts previews that replace no named entity
  of the file, or one already replaced.
* **Interaction.** The view changes halfway through the refinement on the 25 Mbit/s arm (a pan and a zoom, `interaction` rows) and
  the job of the old generation stops at its next checkpoint. The discard share is the decoded bytes thrown away over the decoded
  bytes. Requests not done when the view changed are cancelled and never decoded; those in flight count as cancelled, so that
  figure is an upper bound of what was saved.
* **Missing.** An arm that cannot run is a row with no value and a note, never an interpolation. None was missing in this run.

## Thresholds

Declared in `tests/issue_64_progressive.rs` and `src/progressive_bench.rs` before the first run. Failure if:

* the directory is not ready within 300 ms on `architectural-1e6` at 25 Mbit/s and 60 ms;
* complete Tier 0 takes over 1.5 s at 25 Mbit/s, or over 4 s at 5 Mbit/s and 200 ms, on `architectural-1e6`;
* any entity of any viewport on any entry and arm is not represented;
* the worst preview error on screen is over 2 px, or a preview has no stable identity;
* full visible refinement takes over 5 s on `architectural-1e6` at 25 Mbit/s;
* a mid-load pan discards more than 20% of the bytes already decoded;
* a stage time varies by more than 20% (range over the median) across three repeat runs.

## Verdicts

Computed from `docs/bench/progressive.csv` by `cargo bench -p pvec-bench --bench progressive -- report`, and held to this list by the
test. A failure is a result of this task; the thresholds were not changed and nothing was tuned to move them.

### linux-x86_64

- directory-300ms on linux-x86_64: FAIL (architectural-1e6 on 25mbit-60ms: 2.1427 s (limit 0.3 s))
- tier0-1500ms on linux-x86_64: FAIL (architectural-1e6 on 25mbit-60ms: 3.3232 s (limit 1.5 s))
- tier0-4s-5mbit on linux-x86_64: FAIL (architectural-1e6 on 5mbit-200ms: 15.8492 s (limit 4 s))
- coverage-complete on linux-x86_64: FAIL (3 of 27 entry and arm rows are below 1.0: huge-coordinate-1e5 on local (0.941935); huge-coordinate-1e5 on 25mbit-60ms (0.941935); huge-coordinate-1e5 on 5mbit-200ms (0.941935))
- preview-2px on linux-x86_64: FAIL (architectural-1e6 on 25mbit-60ms: 6.1886 px (limit 2 px))
- preview-identity on linux-x86_64: PASS (27 rows, every preview replaces a named entity)
- refine-5s on linux-x86_64: FAIL (architectural-1e6 on 25mbit-60ms: 10.3745 s (limit 5 s))
- pan-discard-20pct on linux-x86_64: FAIL (architectural-1e6 on 25mbit-60ms: 0.4374 of decoded bytes (limit 0.2 of decoded bytes))
- repeat-spread-20pct on linux-x86_64: FAIL (5 of 108 stage times vary by more than 20%: architectural-1e3 local directory_s (54.3%); architectural-1e3 local refine_first_s (44.5%); architectural-1e3 local refine_full_s (44.5%); architectural-1e3 local tier0_s (41.7%); pathological-1e3 local directory_s (55.0%))

### linux-aarch64

- directory-300ms on linux-aarch64: FAIL (architectural-1e6 on 25mbit-60ms: 2.1440 s (limit 0.3 s))
- tier0-1500ms on linux-aarch64: FAIL (architectural-1e6 on 25mbit-60ms: 3.3251 s (limit 1.5 s))
- tier0-4s-5mbit on linux-aarch64: FAIL (architectural-1e6 on 5mbit-200ms: 15.8511 s (limit 4 s))
- coverage-complete on linux-aarch64: FAIL (3 of 27 entry and arm rows are below 1.0: huge-coordinate-1e5 on local (0.941935); huge-coordinate-1e5 on 25mbit-60ms (0.941935); huge-coordinate-1e5 on 5mbit-200ms (0.941935))
- preview-2px on linux-aarch64: FAIL (architectural-1e6 on 25mbit-60ms: 6.1886 px (limit 2 px))
- preview-identity on linux-aarch64: PASS (27 rows, every preview replaces a named entity)
- refine-5s on linux-aarch64: FAIL (architectural-1e6 on 25mbit-60ms: 10.3830 s (limit 5 s))
- pan-discard-20pct on linux-aarch64: FAIL (architectural-1e6 on 25mbit-60ms: 0.4374 of decoded bytes (limit 0.2 of decoded bytes))
- repeat-spread-20pct on linux-aarch64: PASS (108 stage times, the widest spread 7.2% (limit 20%))

### What the verdicts say

* **Directory (fail, both platforms).** 2.14 s against 0.3 s at 25 Mbit/s on `architectural-1e6`. The directory stage reads about
  5.7 MB per viewport (mean `bytes_directory` 5,739,088, in 5 ranges), which is 1.8 s of link before the round trips. The index and
  directory are too big to be "ready" inside 300 ms on that link, whatever the CPU does (local p95 is 6 ms).
* **Tier 0 (fail, both platforms).** 3.32 s against 1.5 s at 25 Mbit/s and 15.85 s against 4 s at 5 Mbit/s on `architectural-1e6`,
  from about 1.54 MB of Tier 0 over about 26 ranges per viewport on top of the directory. On the smaller entries Tier 0 is inside
  1.5 s at 25 Mbit/s (0.25 s to 1.37 s) except `spline-heavy-1e5` (7.5 s) and `spline-heavy-1e6` (74.8 s); those are not the
  judged entry but are in the table below. At `spline-heavy-1e6` the local p95 is 2.9 s on x64 and 5.5 s on arm64, so the
  curve-heavy Tier 0 is CPU bound as well as link bound.
* **Coverage (fail, both platforms).** `huge-coordinate-1e5` represents 0.9419 of the entities that intersect its worst viewport
  on all three arms, on both platforms (3 of 27 entry and arm rows). Every other entry and arm is 1.0. Because the number is the
  same on the local arm it is not a network effect. This task does not find the cause; it is for P8.6.
* **Preview error (fail, both platforms).** The worst recorded viewport shows a preview with an error bound of 6.19 px on
  `architectural-1e6` (7.9 px on several 10^5 and 10^6 entries) against 2 px. The same previews are 0.19 to 0.25 px at the fit zoom,
  inside the 0.5 px budget the writer is given, so the budget holds where it is set and does not hold once the viewer zooms in
  before the exact refinement arrives. These previews are display fidelity, not geometric exactness.
* **Preview identity (pass, both platforms).** Every preview replaces a named entity, none is anonymous and none is replaced twice.
* **Refinement (fail, both platforms).** 10.37 s (x64) and 10.38 s (arm64) against 5 s at 25 Mbit/s on `architectural-1e6`;
  about 5.5 MB of exact data over about 7 ranges per viewport. At 5 Mbit/s the same figure is 51 s.
* **Pan (fail, both platforms).** A pan halfway through the refinement discards 43.7% of the decoded bytes (the worst viewport 91.8%)
  against 20%; a zoom discards 68.9% (the worst viewport 100%). On average 3.4 requests and about 258 kB are cancelled before decoding.
* **Repeat spread (fail on x64, pass on arm64).** On arm64 the widest spread is 7.2%. On x64 five of 108 stage times pass 20%, all of them
  on the `local` arm of the 10^3 entries, where a stage is 10 to 100 microseconds and a single scheduler hiccup is a large share:
  `architectural-1e3` directory 54.3%, first refinement 44.5%, full refinement 44.5%, Tier 0 41.7%, and `pathological-1e3` directory
  55.0%. No modelled-network stage and no entry of 10^5 or more is over 20% on either platform. It is recorded as measured: the
  threshold says any stage time, and the run did not exclude the noisy ones.

## Stage times (p95, seconds)

Native x64 (MARS) and arm64 (mercury) agree to within a few percent wherever the link dominates, because the link is modelled and
identical on both; they differ where the CPU matters (`spline-heavy`, local arm).

| entry | directory, 25 Mbit/s | Tier 0, 25 Mbit/s | Tier 0, 5 Mbit/s | full refinement, 25 Mbit/s | Tier 0, local, x64 | Tier 0, local, arm64 | worst preview px | min coverage |
|---|---|---|---|---|---|---|---|---|
| pathological-1e3 | 0.30 | 0.71 | 2.94 | 0.72 | 0.00 | 0.00 | 0.00 | 1.000 |
| architectural-1e3 | 0.18 | 0.25 | 0.83 | 0.26 | 0.00 | 0.00 | 7.67 | 1.000 |
| architectural-1e5 | 0.49 | 0.70 | 2.86 | 1.65 | 0.00 | 0.00 | 7.80 | 1.000 |
| mep-1e5 | 0.49 | 0.88 | 3.72 | 1.94 | 0.00 | 0.00 | 7.92 | 1.000 |
| industrial-1e5 | 0.50 | 1.15 | 5.04 | 2.13 | 0.00 | 0.00 | 7.92 | 1.000 |
| huge-coordinate-1e5 | 0.50 | 1.37 | 6.16 | 2.36 | 0.00 | 0.00 | 0.00 | 0.942 |
| spline-heavy-1e5 | 0.32 | 7.51 | 37.01 | 7.80 | 0.04 | 0.07 | 7.92 | 1.000 |
| architectural-1e6 | 2.14 | 3.32 | 15.85 | 10.37 | 0.02 | 0.02 | 6.19 | 1.000 |
| spline-heavy-1e6 | 1.55 | 74.78 | 361.75 | 74.78 | 2.93 | 5.53 | 7.93 | 1.000 |

The figures in the table are native x64 except the arm64 Tier 0 local column; the arm64 rows are all in the CSV.

## What this does not do

* It does not tune. The thresholds, the arms and the model were fixed before the run, and nothing was adjusted afterwards.
  Tuning is P8.6 (pvec#65) and what the freeze does about each miss is P8.7 (pvec#66).
* It does not fetch over a real network. The link is a model, so read absolute network seconds as the model's answer to the
  recorded byte counts, and read the CPU part as the host's.
* It does not test geometric exactness. That is the oracle and the fidelity reports; a preview here is display fidelity only.
* Hosted CI is off, so the evidence is these two hosts and the gate comment on the pull request.
