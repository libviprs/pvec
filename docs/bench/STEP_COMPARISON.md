# PVec against STEP Part 21 and STEP-HDF (P8.2)

This is the measurement that says whether `.pvec` earns its existence beside the two ISO encodings, repeated on the shipped
implementation and the pinned corpus because P8.7's container freeze rests on it (panel finding 3: benchmark STEP-HDF before
inventing the final container). The harness is `crates/pvec-bench/benches/vs_step.rs`, the STEP-HDF arm is `spikes/step-hdf`
(`src/corpus.rs`), the independent Part 21 reader is the `tools/step-eval` image, `tools/step-compare.sh` drives all three on one
host, and the raw numbers are `docs/bench/step_comparison.csv` (one row per arm, entry and platform; the `key=value` files behind
it are in `docs/bench/step_comparison/`). `crates/pvec-bench/tests/issue_61_vs_step.rs` holds this document to that CSV.

## Inputs, arms and platforms

* Corpus: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`; the 22 known-truth fixtures and the ten generated
  entries, with the 200 recorded viewports of `viewports.csv` per entry. `architectural-1e7` is a row in the CSV with the `.pvec` arm
  measured and the other two arms reported missing: its Part 21 text would be about 3 GB and the HDF file and the evaluator cannot
  hold it. I did not try `spline-heavy-1e6` on arm64 (an 835 MB text, the shapes and the curves alive at once, on a 12 GiB VM), so the arm64 rows stop at 10^6 architectural.
* Identical geometry. All three arms start from the one `.pvec` of the entry: its shapes are read back through the reference reader,
  exported to Part 21 through the P6.2 writer, and dumped for the HDF toolchain. The CSV carries the FNV-1a digest of that shape list
  in the `geometry` column and the three arms agree on it for every entry, with the same entity count where an arm has one.
* **Arm 1, `.pvec` as implemented.** The container as written by the deterministic writer with its default options (no compression),
  read through `ContainerReader` over a file, with the spatial directory and the viewport plan of `pvec-stream`.
* **Arm 2, ISO 10303-21 Part 21 text** from P6.2 (`pvec-interchange`), read by P6.3's independent evaluator: OpenCASCADE 7.6 through
  Debian's FreeCAD module, image `ghcr.io/libviprs/pvec-step-eval@sha256:957058fc03489500ae9866b795ba4ece8dc43305042c916731908d256c07c0dc`
  (a multi-arch index; run with `--network none --read-only`). Its cold open is OCCT's `Part.read`; the textual 2D to 3D lift the
  evaluator needs is timed apart and not counted. The OCCT arm stops at 10^5 entities. A second, in-process reference parse of the
  text (instances to values, nothing else, `src/part21.rs`) is the floor under any Part 21 reader and is in the notes of each row.
* **Arm 3, ISO/TS 10303-26 STEP-HDF** with the phase 1 spike toolchain (`spikes/step-hdf`, libhdf5 1.10.8 through `hdf5-metno`),
  H2 layout, re-run at production scale. As in phase 1 it is not claimed to conform to Part 26 (the text was not available), and it
  has no spatial index: every viewport scans the bounds table. Three configurations are written (contiguous; chunk 65,536 + shuffle +
  deflate 6; chunk 4,096 + shuffle + deflate 6 with the newest format), the chunk cache is raised to 256 MiB, and the **compressed size
  is the smaller of the two compressed files and the viewport numbers those of the configuration with the best p95**. Both choices
  favour STEP-HDF; that is the strict way to judge `.pvec` against it.
* Baselines, declared before the run. Arm 2 is the **interchange baseline**. Arm 3 is the **binary-container baseline**. Phase 1's
  spike numbers are the **regression baseline** for arm 3: the unchanged spike code reproduces its phase 1 browser footprint to within a
  byte (152,669 against 152,668 gzipped, measured again on this toolchain) and its H2 layout hands every corpus entry back bit for bit
  (`roundtrip_exact` in the raw files); the phase 1 `million` fixture is a different drawing, so its sizes are not compared with the
  corpus entries'.
* Platforms. Every row says its platform and the CSV is the only place numbers meet. **native x64** is MARS (Intel, the container
  run with `--platform linux/amd64`, image `pvec-rust:1.99.1`, id `sha256:fd085f9e06b8d884d06c57bf9c7f5f77c27c1e06c19969834a893565e63a7de6`,
  rustc 1.99.1). **arm64** is mercury (Apple silicon, the Colima VM, `--platform linux/arm64`, an image built there from the same recipe).
  No arm64 number is compared with an x64 number: each platform is judged from its own rows, and the two differ in places the tables show
  (the Part 21 text is 286,373,450 bytes on x64 and 286,372,662 on arm64, because the rotated ellipse axes come from the C library's
  `sin` and `cos`, which round differently). Both hosts were shared and loaded (load averages 2 to 13), so read times as ratios.

## What is measured

* **Encoded size**, raw (the file as written) and compressed under each arm's own compression: `.pvec` the smaller of LZ4 and deflate
  applied per block by the writer; Part 21 the whole file deflated at level 6; STEP-HDF as above.
* **Cold open to directory-ready.** `.pvec`: open the reader and parse the spatial directory and the straddler index (median of five).
  Part 21: OCCT's read of the whole file. STEP-HDF: open the file and read the roots and bounds tables every query starts from.
* **Viewport random access, p50 and p95, over the 200 recorded viewports**, each on a fresh reader (a fresh open per viewport, the
  phase 1 method), from a file in the page cache. Part 21 has no random access, so each viewport costs the full read.
* **Bytes for one viewport**: everything the cold reader asked of the file from its first byte to the viewport drawn, the open and the
  directory included. The CSV column is the **maximum** over the 200 recorded viewports, which includes the whole-drawing one, so the
  figure is deliberately the strict one; for STEP-HDF it is the library's own reads from its log driver.
* **Peak RSS** (`VmHWM`) of the process that did the reading.
* **Implementation complexity**: the non-blank, non-comment lines of the decode path, and the compressed WebAssembly size of the decode
  artefact (the `(artefact)` rows).

## Thresholds

Declared in `tests/issue_61_vs_step.rs` and `src/step_bench.rs` before the first run. Failure if:

* `.pvec` cold open to directory-ready is not at least 10x faster than a full Part 21 parse on `architectural-1e5`;
* one viewport transfers more than 20% of the Part 21 file on `architectural-1e6` (the whole file is the honest comparison, since Part
  21 has no range story);
* `.pvec` compressed exceeds STEP-HDF's compressed size on the same geometry by more than 10%, on any entry;
* `.pvec` viewport p95 exceeds STEP-HDF's on any entry;
* the `pvec-wasm` decode artefact exceeds 500 KiB compressed, or STEP-HDF's browser footprint is not reported alongside it.

A missed threshold is a result and is recorded below; P8.7 must then decide explicitly.

## Results

- cold-open-10x on linux-aarch64: PASS (architectural-1e5: Part 21 read 9717.8 ms against .pvec directory-ready 0.406 ms, 23921.2x (needs 10x))
- viewport-bytes-20pct on linux-aarch64: PASS (architectural-1e6: the largest recorded viewport moves 55056288 bytes, 19.23% of the 286372662 byte Part 21 file (limit 20%))
- compressed-size-110pct on linux-aarch64: PASS (30 entries compared, none above 10% over STEP-HDF)
- viewport-p95-vs-hdf on linux-aarch64: PASS (30 entries compared, .pvec p95 never above STEP-HDF's)
- wasm-500kib on linux-aarch64: PASS (decode artefact 83.4 KiB compressed (limit 500 KiB))
- step-hdf-footprint-reported on linux-aarch64: PASS (browser footprint 149.1 KiB compressed)
- cold-open-10x on linux-x86_64: PASS (architectural-1e5: Part 21 read 7490.9 ms against .pvec directory-ready 0.504 ms, 14869.7x (needs 10x))
- viewport-bytes-20pct on linux-x86_64: PASS (architectural-1e6: the largest recorded viewport moves 55056288 bytes, 19.23% of the 286373450 byte Part 21 file (limit 20%))
- compressed-size-110pct on linux-x86_64: PASS (31 entries compared, none above 10% over STEP-HDF)
- viewport-p95-vs-hdf on linux-x86_64: FAIL (1 of 31 entries where .pvec p95 is above STEP-HDF's, as the ratio: spline-heavy-1e6 (10.732))
- wasm-500kib on linux-x86_64: PASS (decode artefact 83.4 KiB compressed (limit 500 KiB))
- step-hdf-footprint-reported on linux-x86_64: PASS (browser footprint 149.1 KiB compressed)
- cold-open-10x on linux-aarch64: PASS (architectural-1e5: Part 21 read 9717.8 ms against .pvec directory-ready 0.406 ms, 23921.2x (needs 10x))
- viewport-bytes-20pct on linux-aarch64: PASS (architectural-1e6: the largest recorded viewport moves 55056288 bytes, 19.23% of the 286372662 byte Part 21 file (limit 20%))
- compressed-size-110pct on linux-aarch64: PASS (30 entries compared, none above 10% over STEP-HDF)
- viewport-p95-vs-hdf on linux-aarch64: PASS (30 entries compared, .pvec p95 never above STEP-HDF's)
- wasm-500kib on linux-aarch64: PASS (decode artefact 83.4 KiB compressed (limit 500 KiB))
- step-hdf-footprint-reported on linux-aarch64: PASS (browser footprint 149.1 KiB compressed)
- cold-open-10x on linux-x86_64: PASS (architectural-1e5: Part 21 read 7490.9 ms against .pvec directory-ready 0.504 ms, 14869.7x (needs 10x))
- viewport-bytes-20pct on linux-x86_64: PASS (architectural-1e6: the largest recorded viewport moves 55056288 bytes, 19.23% of the 286373450 byte Part 21 file (limit 20%))
- compressed-size-110pct on linux-x86_64: PASS (31 entries compared, none above 10% over STEP-HDF)
- viewport-p95-vs-hdf on linux-x86_64: FAIL (1 of 31 entries where .pvec p95 is above STEP-HDF's, as the ratio: spline-heavy-1e6 (10.732))
- wasm-500kib on linux-x86_64: PASS (decode artefact 83.4 KiB compressed (limit 500 KiB))
- step-hdf-footprint-reported on linux-x86_64: PASS (browser footprint 149.1 KiB compressed)

### What the verdicts say

**Cold open: 15,000x on x64, 24,000x on arm64, against the independent evaluator.** OCCT takes 7.5 s (x64) and 9.7 s (arm64) to read
the 27 MB Part 21 text of `architectural-1e5`; the `.pvec` is directory-ready in 0.50 and 0.41 ms. Even the floor, the in-process parse
that does nothing but turn the text into values, takes 89 ms on x64 (and 11 ms to read the file), 176 times the `.pvec` open. STEP-HDF
opens as fast as `.pvec` (0.43 ms against 0.50), because both read a small amount; its cost comes after the open.

**One viewport moves at most 19.2% of the Part 21 file, which is a pass by 0.8 points.** The largest of the 200 recorded viewports on
`architectural-1e6` is the whole drawing, 55.06 MB, against 286.4 MB of text. That number is the file size ratio and not a property of
the index: the middle viewports move far less, but the threshold is stated on the worst one, so it is the whole-drawing viewport that
sets it, and it would fail the day the `.pvec` of this drawing grew by 4%.

**Compressed, `.pvec` is smaller than STEP-HDF on all 31 entries where both ran.** On `architectural-1e6` it is 31.4 MB against
45.7 MB (0.69) and on `spline-heavy-1e5` 11.1 MB against 15.6 MB; on the 22 fixtures, a few hundred bytes each, HDF's per-dataset
metadata puts it at 4.8 KB or more. Part 21 deflated is bigger than `.pvec` on every entry except nine of the spline-carrying
fixtures (`bezier-d3-001` and the B-spline and NURBS ones), where the text of one curve deflates to about 550 bytes against about
950 for the container's block overheads.

**Viewport p95: `.pvec` is ahead everywhere except `spline-heavy-1e6`, where it is 10.7x behind on x64.** At 10^5 the `.pvec` p95 is 3.2 to
4.9 ms against 24 to 36 ms for STEP-HDF (6 to 11x), and at `architectural-1e6` 31 ms against 123 ms (4x). On the two spline-heavy entries the
margin shrinks (51 against 79 ms at 10^5) and then reverses: 3,189 ms against 297 ms at 10^6. The cause is in the bytes column: a viewport
that touches the advanced curve set reads all of it (`docs/STREAMING.md` 2.8, and 244 MB of the 244 MB file here), while STEP-HDF scans
a 32 MB bounds table and gathers only the matching rows. That is the cost `STREAMING.md` 2.8 names, in numbers. Reading less of the advanced set is not what pvec#159 (one contiguous range per chunk)
does, so it is a question for P8.6 and P8.7, and the arm64 rows, which do not include `spline-heavy-1e6`, cannot say whether it shows there too.

**The decode artefact is 83.4 KiB compressed against a 500 KiB limit, and STEP-HDF's browser footprint is 149.1 KiB.** The `pvec-wasm`
crate (P7.3) does not exist yet, so the artefact measured is a stand-in, `spikes/pvec-wasm-standin`: the container reader, the spatial
directory, the viewport plan and both codecs behind two exports, wasm32-unknown-unknown, size-optimised and stripped (208,044 bytes
raw, 85,417 gzipped). It was instantiated and run under Node 22 on `architectural-1e3` and decoded all 1,000 entities. The real artefact
will carry more (a public API, the semantic layers), and 83 KiB is the figure to add to. For STEP-HDF the only reader that builds for the
browser is the pure-Rust `hdf5-reader` with a decode harness (483,308 bytes, 152,669 gzipped, newest-format files only); libhdf5 itself
does not build for wasm32 (phase 1, `docs/spikes/01-4-step-hdf.md` section 5).

**Complexity.** The `.pvec` decode path files are 6,158 non-blank, non-comment lines (the writer-side code that shares those files
included, so this is an upper bound). The Part 21 reference parse is 179 lines, and it parses text and does no geometry: the independent
evaluator is OpenCASCADE, which is millions of lines in a container. The STEP-HDF H2 reader is 226 lines of Rust over a 3.9 MB C library
with 34 shared-library dependencies, or the pure-Rust reader's 40-crate tree.

## Per entry, native x64

Sizes in bytes, times in milliseconds, `open` is cold open to directory-ready (`NA` where the arm could not run: the OCCT evaluator stops
at 10^5 entities, and the 10^7 case has no Part 21 or HDF file).

| entry | entities | .pvec bytes | Part 21 bytes | HDF bytes | .pvec cmp | Part 21 cmp | HDF cmp | open pvec / P21 / HDF ms | p95 pvec / HDF ms | max viewport bytes pvec / HDF |
|---|---|---|---|---|---|---|---|---|---|---|
| arc-seam-001 | 1 | 456 | 801 | 6560 | 416 | 509 | 4788 | 0.0037 / 0.6880 / 0.1588 | 0.0041 / 0.3342 | 608 / 8008 |
| bezier-d3-001 | 1 | 1088 | 834 | 7704 | 904 | 501 | 6845 | 0.0011 / 0.7300 / 0.1511 | 0.0032 / 0.4311 | 1888 / 9986 |
| bspline-d3-001 | 1 | 1160 | 973 | 7704 | 944 | 549 | 6845 | 0.0029 / 0.8610 / 0.1516 | 0.0068 / 0.4324 | 2032 / 10058 |
| bspline-d5-001 | 1 | 1224 | 1049 | 7704 | 952 | 564 | 6845 | 0.0030 / 0.8100 / 0.0679 | 0.0071 / 0.3746 | 2160 / 30098 |
| circle-001 | 1 | 440 | 719 | 6560 | 408 | 471 | 4788 | 0.0034 / 0.7460 / 0.1476 | 0.0017 / 0.2785 | 592 / 7992 |
| closed-by-endpoint-001 | 1 | 1168 | 971 | 7704 | 936 | 539 | 6845 | 0.0028 / 1.18 / 0.1504 | 0.0070 / 0.4142 | 2040 / 10058 |
| closed-periodic-001 | 1 | 1216 | 1171 | 7704 | 1000 | 585 | 6845 | 0.0030 / 0.7410 / 0.1459 | 0.0070 / 0.4229 | 2136 / 10106 |
| ellipse-001 | 1 | 456 | 740 | 6560 | 424 | 479 | 4788 | 0.0030 / 0.7560 / 0.1013 | 0.0037 / 0.1639 | 624 / 5305 |
| elliptical-arc-001 | 1 | 480 | 827 | 6560 | 448 | 520 | 4788 | 0.0031 / 1.07 / 0.0667 | 0.0017 / 0.1531 | 648 / 5294 |
| large-coords-001 | 1 | 480 | 838 | 6560 | 432 | 468 | 4788 | 0.0031 / 0.9790 / 0.0875 | 0.0042 / 0.1686 | 664 / 17692 |
| line-001 | 1 | 432 | 833 | 6560 | 400 | 532 | 4788 | 0.0016 / 0.9910 / 0.0687 | 0.0017 / 0.2650 | 576 / 5245 |
| lwpolyline-closed-001 | 1 | 496 | 804 | 6560 | 432 | 476 | 4788 | 0.0034 / 0.7380 / 0.1490 | 0.0037 / 0.3312 | 696 / 8056 |
| nonunit-weight-001 | 1 | 1160 | 1030 | 7976 | 960 | 555 | 7113 | 0.0027 / 0.7420 / 0.1463 | 0.0065 / 0.2040 | 2024 / 10554 |
| nurbs-d2-offconic-001 | 1 | 1160 | 1029 | 7976 | 976 | 564 | 7113 | 0.0017 / 0.7070 / 0.0618 | 0.0064 / 0.3199 | 2024 / 10554 |
| nurbs-d3-001 | 1 | 1192 | 1074 | 7976 | 984 | 578 | 7113 | 0.0026 / 0.7700 / 0.0602 | 0.0064 / 0.2270 | 2096 / 10594 |
| polyline-001 | 1 | 488 | 808 | 6560 | 424 | 481 | 4788 | 0.0034 / 0.7170 / 0.0688 | 0.0036 / 0.2708 | 688 / 17668 |
| polyline2d-001 | 1 | 488 | 808 | 6560 | 432 | 482 | 4788 | 0.0031 / 0.6760 / 0.0613 | 0.0040 / 0.1425 | 688 / 8056 |
| repeated-knot-c0-001 | 1 | 1168 | 958 | 7704 | 936 | 537 | 6845 | 0.0028 / 0.7150 / 0.0704 | 0.0034 / 0.2437 | 2040 / 30078 |
| tiny-radius-001 | 1 | 448 | 718 | 6560 | 424 | 469 | 4788 | 0.0017 / 0.6800 / 0.0689 | 0.0024 / 0.1772 | 600 / 15583 |
| bulge-arc-001 | 2 | 504 | 1141 | 6560 | 464 | 597 | 4788 | 0.0017 / 0.6950 / 0.1475 | 0.0016 / 0.3062 | 704 / 8088 |
| bulge-arc-neg-001 | 2 | 512 | 1164 | 6560 | 480 | 608 | 4788 | 0.0030 / 0.7140 / 0.0727 | 0.0037 / 0.1546 | 712 / 15609 |
| layer-state-001 | 3 | 712 | 1325 | 6560 | 584 | 680 | 4788 | 0.0025 / 0.7250 / 0.1568 | 0.0037 / 0.1346 | 1008 / 8144 |
| architectural-1e3 | 1000 | 55640 | 246906 | 88616 | 32784 | 50585 | 60061 | 0.0087 / 24.73 / 0.1522 | 0.0766 / 1.27 | 59464 / 92242 |
| pathological-1e3 | 1000 | 1116040 | 3295883 | 1153884 | 513248 | 758896 | 673990 | 0.0095 / 832 / 0.0624 | 0.7475 / 2.61 | 1125396 / 1158400 |
| architectural-1e5 | 100000 | 5492896 | 27302244 | 8062146 | 3123656 | 5145855 | 4575957 | 0.5038 / 7491 / 0.4292 | 3.18 / 24.21 | 5617000 / 8066662 |
| huge-coordinate-1e5 | 100000 | 9375352 | 41507577 | 12553082 | 5561608 | 9330642 | 6615472 | 0.3461 / 8386 / 0.3717 | 4.88 / 29.04 | 9379176 / 12557598 |
| industrial-1e5 | 100000 | 6731160 | 28414674 | 9253700 | 4139472 | 5783590 | 6271868 | 0.5549 / 7656 / 0.3821 | 3.39 / 36.42 | 6734984 / 9258216 |
| mep-1e5 | 100000 | 6756960 | 28394896 | 9242498 | 3856384 | 5863669 | 5808876 | 0.5253 / 8823 / 0.4515 | 3.47 / 35.27 | 6760792 / 9247014 |
| spline-heavy-1e5 | 100000 | 24413992 | 79877804 | 30673550 | 11111024 | 18679733 | 15648365 | 0.3595 / 10950 / 0.2990 | 50.77 / 78.67 | 24417816 / 30678066 |
| architectural-1e6 | 1000000 | 55052464 | 286373450 | 80683324 | 31383656 | 51831280 | 45653157 | 4.17 / NA / 5.30 | 30.96 / 123 | 55056288 / 80687840 |
| spline-heavy-1e6 | 1000000 | 244437392 | 834774113 | 307260052 | 109385080 | 189231894 | 153522069 | 4.28 / NA / 8.16 | 3189 / 297 | 244441216 / 307264568 |
| architectural-1e7 | 10000000 | 550378792 | NA | NA | 313538392 | NA | NA | 55.96 / NA / NA | 895 / NA | 550382612 / NA |


## Per entry, arm64

| entry | entities | .pvec bytes | Part 21 bytes | HDF bytes | .pvec cmp | Part 21 cmp | HDF cmp | open pvec / P21 / HDF ms | p95 pvec / HDF ms | max viewport bytes pvec / HDF |
|---|---|---|---|---|---|---|---|---|---|---|
| arc-seam-001 | 1 | 456 | 801 | 6560 | 416 | 509 | 4788 | 0.0021 / 2.36 / 0.0760 | 0.0026 / 0.1635 | 608 / 8008 |
| bezier-d3-001 | 1 | 1088 | 834 | 7704 | 904 | 501 | 6845 | 0.0016 / 0.8520 / 0.0769 | 0.0043 / 0.2277 | 1888 / 9986 |
| bspline-d3-001 | 1 | 1160 | 973 | 7704 | 944 | 549 | 6845 | 0.0017 / 0.7590 / 0.0739 | 0.0045 / 0.2259 | 2032 / 10058 |
| bspline-d5-001 | 1 | 1224 | 1049 | 7704 | 952 | 564 | 6845 | 0.0019 / 0.7700 / 0.0771 | 0.0045 / 0.2247 | 2160 / 10122 |
| circle-001 | 1 | 440 | 719 | 6560 | 408 | 471 | 4788 | 0.0021 / 0.7360 / 0.0756 | 0.0024 / 0.1659 | 592 / 7992 |
| closed-by-endpoint-001 | 1 | 1168 | 971 | 7704 | 936 | 539 | 6845 | 0.0020 / 0.7580 / 0.0784 | 0.0046 / 0.2243 | 2040 / 10058 |
| closed-periodic-001 | 1 | 1216 | 1171 | 7704 | 1000 | 585 | 6845 | 0.0017 / 0.7920 / 0.0744 | 0.0044 / 0.2292 | 2136 / 10106 |
| ellipse-001 | 1 | 456 | 740 | 6560 | 424 | 479 | 4788 | 0.0021 / 0.7370 / 0.0734 | 0.0025 / 0.1647 | 624 / 8024 |
| elliptical-arc-001 | 1 | 480 | 827 | 6560 | 448 | 520 | 4788 | 0.0020 / 0.7990 / 0.0779 | 0.0025 / 0.1669 | 648 / 8024 |
| large-coords-001 | 1 | 480 | 838 | 6560 | 432 | 468 | 4788 | 0.0020 / 0.8370 / 0.0749 | 0.0023 / 0.1706 | 664 / 8040 |
| line-001 | 1 | 432 | 833 | 6560 | 400 | 532 | 4788 | 0.0021 / 0.7460 / 0.0749 | 0.0025 / 0.1645 | 576 / 8000 |
| lwpolyline-closed-001 | 1 | 496 | 804 | 6560 | 432 | 476 | 4788 | 0.0020 / 0.7210 / 0.0734 | 0.0024 / 0.1736 | 696 / 8056 |
| nonunit-weight-001 | 1 | 1160 | 1030 | 7976 | 960 | 555 | 7113 | 0.0017 / 0.7940 / 0.0726 | 0.0045 / 0.2379 | 2024 / 10554 |
| nurbs-d2-offconic-001 | 1 | 1160 | 1029 | 7976 | 976 | 564 | 7113 | 0.0016 / 0.7580 / 0.0767 | 0.0046 / 0.2430 | 2024 / 10554 |
| nurbs-d3-001 | 1 | 1192 | 1074 | 7976 | 984 | 578 | 7113 | 0.0016 / 0.7830 / 0.0736 | 0.0045 / 0.2388 | 2096 / 10594 |
| polyline-001 | 1 | 488 | 808 | 6560 | 424 | 481 | 4788 | 0.0020 / 0.7270 / 0.0725 | 0.0025 / 0.1763 | 688 / 8056 |
| polyline2d-001 | 1 | 488 | 808 | 6560 | 432 | 482 | 4788 | 0.0025 / 0.7520 / 0.0752 | 0.0026 / 0.1768 | 688 / 8056 |
| repeated-knot-c0-001 | 1 | 1168 | 958 | 7704 | 936 | 537 | 6845 | 0.0017 / 0.7980 / 0.0761 | 0.0045 / 0.2256 | 2040 / 10058 |
| tiny-radius-001 | 1 | 448 | 718 | 6560 | 424 | 469 | 4788 | 0.0020 / 0.7030 / 0.0721 | 0.0025 / 0.1634 | 600 / 7992 |
| bulge-arc-001 | 2 | 504 | 1141 | 6560 | 464 | 597 | 4788 | 0.0020 / 0.7690 / 0.0777 | 0.0023 / 0.1600 | 704 / 8088 |
| bulge-arc-neg-001 | 2 | 512 | 1164 | 6560 | 480 | 608 | 4788 | 0.0022 / 0.7950 / 0.0746 | 0.0024 / 0.1628 | 712 / 8088 |
| layer-state-001 | 3 | 712 | 1325 | 6560 | 584 | 680 | 4788 | 0.0015 / 0.7890 / 0.0754 | 0.0025 / 0.1656 | 1008 / 8144 |
| architectural-1e3 | 1000 | 55640 | 246904 | 88616 | 32784 | 50585 | 59898 | 0.0058 / 31.25 / 0.0745 | 0.0525 / 1.57 | 59464 / 92242 |
| pathological-1e3 | 1000 | 1116040 | 3295883 | 1153884 | 513248 | 758896 | 674009 | 0.0063 / 932 / 0.0768 | 0.5067 / 3.03 | 1125396 / 1158400 |
| architectural-1e5 | 100000 | 5492896 | 27302184 | 8062146 | 3123656 | 5145801 | 4575954 | 0.4062 / 9718 / 0.4986 | 3.33 / 27.09 | 5617000 / 8066662 |
| huge-coordinate-1e5 | 100000 | 9375352 | 41507407 | 12553082 | 5561608 | 9330528 | 6615463 | 0.4246 / 11038 / 0.3477 | 6.18 / 37.89 | 9379176 / 12557598 |
| industrial-1e5 | 100000 | 6731160 | 28414350 | 9253700 | 4139472 | 5783328 | 6271888 | 0.4530 / 9600 / 0.3484 | 4.01 / 40.03 | 6734984 / 9258216 |
| mep-1e5 | 100000 | 6756960 | 28394896 | 9242498 | 3856384 | 5863669 | 5808864 | 0.4323 / 9826 / 0.3029 | 4.05 / 39.11 | 6760792 / 9247014 |
| spline-heavy-1e5 | 100000 | 24413992 | 79877804 | 30673550 | 11111024 | 18679733 | 15648349 | 0.2826 / 14012 / 0.5089 | 72.45 / 90.54 | 24417816 / 30678066 |
| architectural-1e6 | 1000000 | 55052464 | 286372662 | 80683324 | 31383656 | 51830606 | 45653056 | 4.64 / NA / 10.18 | 35.91 / 144 | 55056288 / 80687840 |


## What is missing, and why

* OCCT cold open above 10^5 entities, the Part 21 and STEP-HDF arms at 10^7, and `spline-heavy-1e6` on arm64: the host cannot hold
  them, and the CSV rows say so. Nothing is extrapolated.
* Peak RSS of the STEP-HDF arm includes libhdf5's chunk cache (256 MiB allowed), and OCCT's includes the Python interpreter; they are
  recorded as measured and not compared with the `.pvec` figure, which is the reader's alone.
* No arm in this task reads a real-world drawing: the architectural, MEP and industrial entries are modelled mixes (`CORPUS.md`).

## Reproduce

```text
tools/step-compare.sh mars   OUT $(entries)     # native x64, linux/amd64
tools/step-compare.sh mercury OUT $(entries)    # native arm64, linux/arm64
cargo bench -p pvec-bench --bench vs_step -- report docs/bench/step_comparison.csv linux-x86_64
```
