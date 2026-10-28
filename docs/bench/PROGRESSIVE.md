# Progressive coverage benchmark (P8.5)

This measures how long a viewer waits before it can show something true, and how much of the drawing is on screen at each stage:
latency to coverage, not a frame rate. It measures and reports only, and no threshold was moved. This file holds the re-run of P8.6 (pvec#65) at the final
configuration of the tuning pass, on mars and mercury: the first run, which found eight of nine verdicts failing on x64, is in the history of
`main`, and `docs/bench/TUNING.md` says what changed in between and which verdicts are still missed. Every missed threshold is a result below
and P8.7 decides what the freeze does about each one.
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
  1920 px screen, over the viewports that show Tier 0. The writer is given a 0.125 px budget at the fit zoom (`PREVIEW_BUDGET_PX`; it was 0.5 px in the first run) (the `fit` rows), and the
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
test. A failure is a result of this task; the thresholds were not changed. The values below are those of the re-run after the tuning pass.

### linux-x86_64

- directory-300ms on linux-x86_64: PASS (architectural-1e6 on 25mbit-60ms: 0.1864 s (limit 0.3 s))
- tier0-1500ms on linux-x86_64: PASS (architectural-1e6 on 25mbit-60ms: 0.7923 s (limit 1.5 s))
- tier0-4s-5mbit on linux-x86_64: PASS (architectural-1e6 on 5mbit-200ms: 3.5353 s (limit 4 s))
- coverage-complete on linux-x86_64: PASS (27 entry and arm rows, coverage 1.0 on every viewport)
- preview-2px on linux-x86_64: PASS (architectural-1e6 on 25mbit-60ms: 1.7802 px (limit 2 px))
- preview-identity on linux-x86_64: PASS (27 rows, every preview replaces a named entity)
- refine-5s on linux-x86_64: PASS (architectural-1e6 on 25mbit-60ms: 4.6193 s (limit 5 s))
- pan-discard-20pct on linux-x86_64: PASS (architectural-1e6 on 25mbit-60ms: 0.0872 of decoded bytes (limit 0.2 of decoded bytes))
- repeat-spread-20pct on linux-x86_64: FAIL (5 of 108 stage times vary by more than 20%: architectural-1e3 local directory_s (161.2%); architectural-1e3 local refine_first_s (60.8%); architectural-1e3 local refine_full_s (60.8%); architectural-1e3 local tier0_s (65.1%); architectural-1e5 local directory_s (38.1%))

### linux-aarch64

- directory-300ms on linux-aarch64: PASS (architectural-1e6 on 25mbit-60ms: 0.1865 s (limit 0.3 s))
- tier0-1500ms on linux-aarch64: PASS (architectural-1e6 on 25mbit-60ms: 0.7943 s (limit 1.5 s))
- tier0-4s-5mbit on linux-aarch64: PASS (architectural-1e6 on 5mbit-200ms: 3.5373 s (limit 4 s))
- coverage-complete on linux-aarch64: PASS (27 entry and arm rows, coverage 1.0 on every viewport)
- preview-2px on linux-aarch64: PASS (architectural-1e6 on 25mbit-60ms: 1.7802 px (limit 2 px))
- preview-identity on linux-aarch64: PASS (27 rows, every preview replaces a named entity)
- refine-5s on linux-aarch64: PASS (architectural-1e6 on 25mbit-60ms: 4.6357 s (limit 5 s))
- pan-discard-20pct on linux-aarch64: PASS (architectural-1e6 on 25mbit-60ms: 0.0872 of decoded bytes (limit 0.2 of decoded bytes))
- repeat-spread-20pct on linux-aarch64: PASS (108 stage times, the widest spread 16.7% (limit 20%))

### What the verdicts say

The re-run after the tuning pass; the changes are in `docs/bench/TUNING.md`. Eight of nine verdicts failed on x64 in the first run and seven on arm64. Now:

* **Directory (pass, both platforms).** 0.186 s (x64) and 0.186 s (arm64) against 0.3 s at 25 Mbit/s on `architectural-1e6` (it was 2.14 s). A leaf of 32,768 entities made the block and
  spatial directory a few kilobytes and three round trips instead of 5.7 MB.
* **Tier 0 (pass, both platforms).** 0.792 s against 1.5 s at 25 Mbit/s and 3.535 s against 4 s at 5 Mbit/s on x64, 0.794 s and 3.537 s on arm64
  (they were 3.32 s and 15.85 s). The Tier 0 blocks are deflated now, which stores less than LZ4 and is what the 5 Mbit/s link waits on. On the smaller entries Tier 0 at 25 Mbit/s is
  0.24 s to 1.00 s; the two spline entries are not the judged entry and are slow
  (`spline-heavy-1e5` 2.65 s, `spline-heavy-1e6` 14.51 s on x64): the curve previews of a drawing made of splines are most of its bytes.
* **Coverage (pass, both platforms).** Every entity of every viewport of every entry and arm is represented. `huge-coordinate-1e5` was 0.9419 on all three arms in the first run: the rounding
  allowance of a preview was taken from the largest absolute coordinate, which at UTM-scale offsets was five times the whole budget, and 12,010 curves had no preview.
* **Preview error (pass, both platforms).** The worst preview on screen is 1.78 px on `architectural-1e6` (it was 6.19 px) and the worst of any entry is 1.98 px (`spline-heavy-1e6`, x64) and 1.98 px (`spline-heavy-1e6`, arm64),
  against 2 px. The `huge-coordinate-1e5` row was 0 in the first run, and that 0 meant no previews at all (`preview_fraction` 0 on every arm), not a perfect one: the real figure is 1.98 px on x64
  and 1.98 px on arm64, which is at most 2.0 px by construction (a preview is held to 0.125 px at the fit zoom and the benchmark's viewports show it up to 16 times larger). These previews are display fidelity, not geometric exactness.
* **Preview identity (pass, both platforms).** Every preview replaces a named entity, none is anonymous and none is replaced twice.
* **Refinement (pass, both platforms).** 4.62 s (x64) and 4.64 s (arm64) against 5 s at 25 Mbit/s on `architectural-1e6` (it was 10.37 s), because the hot blocks are deflated and the bytes of a viewport are fewer.
  At 5 Mbit/s it is 22.4 s.
* **Pan (pass, both platforms).** A pan halfway through the refinement discards 8.7% of the decoded bytes against 20% (it was 43.7%); a zoom discards 14.9% (it was 68.9%).
* **Repeat spread (fail on x64, pass on arm64).** On x64 five stage times pass 20%, all of them on the `local` arm of the 10^3 and 10^5 entries, where a stage is 10 to 100 microseconds and a single
  scheduler hiccup is a large share. No modelled-network stage and no entry of 10^6 is over 20% on either platform. It is recorded as measured: the threshold says any stage time, and the run did not exclude the noisy ones.

## Stage times (p95, seconds)

Native x64 (MARS) and arm64 (mercury) agree to within a few percent wherever the link dominates, because the link is modelled and
identical on both; they differ where the CPU matters (`spline-heavy`, local arm).

| entry | directory, 25 Mbit/s | Tier 0, 25 Mbit/s | Tier 0, 5 Mbit/s | full refinement, 25 Mbit/s | Tier 0, local, x64 | Tier 0, local, arm64 | worst preview px | min coverage |
|---|---|---|---|---|---|---|---|---|
| pathological-1e3 | 0.30 | 0.60 | 2.40 | 0.60 | 0.00 | 0.00 | 0.00 | 1.000 |
| architectural-1e3 | 0.18 | 0.24 | 0.82 | 0.25 | 0.00 | 0.00 | 1.97 | 1.000 |
| architectural-1e5 | 0.18 | 0.34 | 1.29 | 0.90 | 0.00 | 0.00 | 1.95 | 1.000 |
| mep-1e5 | 0.18 | 0.45 | 1.85 | 1.12 | 0.00 | 0.00 | 1.98 | 1.000 |
| industrial-1e5 | 0.18 | 1.00 | 4.57 | 1.64 | 0.01 | 0.01 | 1.98 | 1.000 |
| huge-coordinate-1e5 | 0.18 | 0.93 | 4.23 | 1.78 | 0.01 | 0.01 | 1.98 | 1.000 |
| spline-heavy-1e5 | 0.20 | 2.65 | 12.79 | 2.92 | 0.01 | 0.02 | 1.98 | 1.000 |
| architectural-1e6 | 0.19 | 0.79 | 3.54 | 4.62 | 0.01 | 0.01 | 1.78 | 1.000 |
| spline-heavy-1e6 | 0.32 | 14.51 | 71.85 | 16.51 | 0.07 | 0.09 | 1.98 | 1.000 |

The figures in the table are native x64 except the arm64 Tier 0 local column; the arm64 rows are all in the CSV. Before the tuning pass the same table had `architectural-1e6` at 2.14, 3.32, 15.85 and 10.37 s, and `huge-coordinate-1e5` at a worst preview of 0.00 px and a coverage of 0.942. Before the tuning pass the same table had `architectural-1e6` at 2.14, 3.32, 15.85 and 10.37 s, and `huge-coordinate-1e5` at a worst preview of 0.00 px and a coverage of 0.942. Before the tuning pass the same table had `architectural-1e6` at 2.14, 3.32, 15.85 and 10.37 s, and `huge-coordinate-1e5` at a worst preview of 0.00 px and a coverage of 0.942.

## What this does not do

* It does not tune. The thresholds, the arms and the model were fixed before the first run, and nothing was adjusted afterwards.
  Tuning is P8.6 (pvec#65, `docs/bench/TUNING.md`) and what the freeze does about each miss is P8.7 (pvec#66).
* It does not fetch over a real network. The link is a model, so read absolute network seconds as the model's answer to the
  recorded byte counts, and read the CPU part as the host's.
* It does not test geometric exactness. That is the oracle and the fidelity reports; a preview here is display fidelity only.
* Hosted CI is off, so the evidence is these two hosts and the gate comment on the pull request.
