# Million-entity streaming results (P5.8, pvec#17)

These are the numbers `tests/streaming_million.rs` measures, from the fixtures in
`tests/streaming_million/harness.rs`. Regenerate a section on the host it is for with
`PVEC_RESULTS_HOST=<name> PVEC_RESULTS_OUT=<file> cargo test --release -p pvec-stream --test streaming_million -- --ignored write_results`.
Time columns are from a release build and are only indicative; the test thresholds are checked on a debug build
with wide tolerances. Sizes, range counts and decode work do not depend on the host.

Each section says where it ran. The first is native x86_64 (MARS, in the `pvec-rust` container); the second is
arm64 (Apple silicon, native, outside a container: it is a reference for throughput, not a gate lane).

## Fixtures and viewports

All three have the same room size and density, so a viewport of a given size holds about the same number of
entities at 100,000 and at 1,000,000 entities. The viewports are 10, 4 and 1 rooms on a side (wide, mid,
tight), about 40,000, 6,400 and 400 entities.

- mixed: every entity a line.
- spline-heavy: one entity in four a cubic B-spline, so 250,000 splines at 1,000,000 entities.
- straddler-heavy: lines plus 60 drawing-spanning lines per row and per column of rooms, so a viewport meets a
  number of them that depends on its size and not on N.

## Peak memory, and the one term that grows (advanced_set_bytes)

Splines can only go in the single advanced set (STREAMING 2.8) and `ContainerReader::advanced()` reads all six
blocks of it for any viewport that touches it, which on the spline-heavy fixture is every viewport. So the
resident bytes there carry a term that grows with N, and the criterion is asserted as `resident -
advanced_set_bytes`. `advanced_set_bytes` is what reading the set added to the reader's peak: the six blocks
and the coalesced request they arrive in, about 1.7 times their decoded size. It is 51 MB on the 1,000,000
entity spline-heavy fixture (30 MB of blocks), 0 on the other two. The follow-up that removes the term is
pvec#148 (chunk the advanced curve set by record block); when it lands the subtraction goes and the plain
criterion returns. Mixed and straddler-heavy have no advanced set, so the growth criterion on `held` is asserted with nothing subtracted.

Taking only that term out is not enough on its own, and I want that to be visible: the spatial directory is
read whole when a file opens (STREAMING chapter 1), it is about 4 bytes per entity, and the reader holds it
twice (the cached block and the parsed copy). That is the `index bytes` column (3.9 MB at 1,000,000 entities,
0.4 MB at 100,000), so `resident - advanced_set_bytes` still grows with N by exactly that. The test budgets it
as a slope (at most 6 bytes per entity) and asserts the growth criterion on `held`, the bytes the viewport pins
besides the indexes and before the advanced set is read, which stays within 35% plus 64 KiB between the two sizes (worst measured: +23% on the mid viewport; the tight
one differs by a few blocks at its edge).

The reader's peak itself is not compared between 100,000 and 1,000,000 entities. Opening reads the directory
through a buffer that costs about 1.2 bytes per entity beyond the index, and that transient is the peak of a small
viewport (mixed tight: 77,584 bytes above the index at 100k, 774,352 at 1M), so a peak comparison could not tell the
viewport from the open. The absolute peak is budgeted instead (16 MiB).

## Cancellation (load shed)

100 viewport changes in 10 s against a source that answers like a slow link (8 ms and 100 MB/s per range, no
block cache). A unit is one block read; a viewport is the units its plan lists (3 x 3 rooms, one room further
along a diagonal each time). Two numbers, because either alone can be gamed:

- shed_rate = units_discarded / units_superseded, at least 0.95. units_superseded counts the units that were in
  flight when their generation was superseded (seen by the unit itself, after its read); units_discarded is
  what P5.6 threw away (`CancelMetrics::work_discarded`). 1.0 means nothing finished for a dead view reached
  the scene.
- useful_fraction = 1 - units_discarded / units_started, at least 0.8 (the panel's prior was 0.5; measured
  0.91 to 0.92). With `MAX_UNITS_AFTER_CANCEL` at 1 each job wastes one unit, so this is 1 - 1/units per job, and
  units per job is the throttle against the change interval (about 11), which is why the units per viewport
  (74) are in the result and not only the ratio.

## The range source in a container

The throttled range source also runs in a hermetic container (pvec#158), as the issue asks. `pvec-hermetic` starts
the pinned `python:3.12-slim` image with no network and a read-only root, the artifact mounted read-only, and the
harness drives `range_server.py` in it through the pipes of one `docker exec -i`. The throttle (8 ms and 100 MB/s per
range) is charged inside the container, and the container is removed when the scenario ends, on a pass and on a
failure. The docker-backed tests are `#[ignore]`d and run in the `docker-tests` stage; the in-process source is
kept for the default run, so both are checked against the same thresholds. The load-shed numbers in the host
sections below are from the in-process source. They do not depend on the host's Docker.

## native x86_64 (mars, release build)

| fixture | entities | viewport | directory ready ms | first drawing ms | ranges | bytes | decode work | blocks | peak resident | index bytes | advanced_set_bytes | resident - advanced | held |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mixed | 100000 | wide | 0.1 | 1.6 | 5 | 1627776 | 3255552 | 628 | 2412064 | 392896 | 0 | 2412064 | 1627776 |
| mixed | 100000 | mid | 0.1 | 0.2 | 3 | 261792 | 523584 | 101 | 722080 | 392896 | 0 | 722080 | 261792 |
| mixed | 100000 | tight | 0.1 | 0.0 | 1 | 18144 | 36288 | 7 | 470480 | 392896 | 0 | 470480 | 18144 |
| mixed | 1000000 | wide | 1.1 | 1.8 | 5 | 1910304 | 3525120 | 680 | 7052400 | 3923856 | 0 | 7052400 | 1762560 |
| mixed | 1000000 | mid | 1.0 | 0.3 | 2 | 347328 | 642816 | 124 | 4698208 | 3923856 | 0 | 4698208 | 321408 |
| mixed | 1000000 | tight | 1.0 | 0.0 | 1 | 31104 | 57024 | 11 | 4698208 | 3923856 | 0 | 4698208 | 28512 |
| spline-heavy | 100000 | wide | 0.1 | 2.9 | 6 | 4228880 | 8457760 | 474 | 7524840 | 295688 | 5705056 | 1819784 | 1228608 |
| spline-heavy | 100000 | mid | 0.1 | 1.8 | 4 | 3202448 | 6404896 | 78 | 6498408 | 295688 | 5948704 | 549704 | 202176 |
| spline-heavy | 100000 | tight | 0.1 | 1.7 | 2 | 3015824 | 6031648 | 6 | 6311784 | 295688 | 5957856 | 353928 | 15552 |
| spline-heavy | 1000000 | wide | 0.8 | 22.6 | 8 | 31433648 | 62685856 | 518 | 56286880 | 2943984 | 50978992 | 5307888 | 1342656 |
| spline-heavy | 1000000 | mid | 0.9 | 21.3 | 5 | 30262064 | 60487840 | 94 | 55187872 | 2943984 | 51663072 | 3524800 | 243648 |
| spline-heavy | 1000000 | tight | 0.9 | 21.4 | 4 | 30023600 | 60042016 | 8 | 54964960 | 2943984 | 51440160 | 3524800 | 20736 |
| straddler-heavy | 100000 | wide | 0.1 | 1.6 | 8 | 1760160 | 3369600 | 650 | 2413128 | 391368 | 0 | 2413128 | 1684800 |
| straddler-heavy | 100000 | mid | 0.1 | 0.3 | 7 | 329728 | 575424 | 111 | 720552 | 391368 | 0 | 720552 | 287712 |
| straddler-heavy | 100000 | tight | 0.1 | 0.1 | 5 | 53856 | 57024 | 11 | 467464 | 391368 | 0 | 467464 | 28512 |
| straddler-heavy | 1000000 | wide | 1.2 | 1.9 | 9 | 2032544 | 3628800 | 700 | 7047344 | 3918800 | 0 | 7047344 | 1814400 |
| straddler-heavy | 1000000 | mid | 1.1 | 0.4 | 7 | 412896 | 689472 | 133 | 4688544 | 3918800 | 0 | 4688544 | 344736 |
| straddler-heavy | 1000000 | tight | 1.0 | 0.1 | 7 | 67040 | 72576 | 14 | 4688544 | 3918800 | 0 | 4688544 | 36288 |

Load shed (100 viewport changes in 10 s, 8 ms + 100 MB/s per range, no block cache): units per viewport 74.0, units_planned 7395, units_started 1292, units_superseded 99, units_discarded 99, jobs cancelled 99; shed_rate 1.000, useful_fraction 0.923; final scene is the last viewport: true.

## arm64 (jupiter, Apple silicon, native outside a container, release build)

| fixture | entities | viewport | directory ready ms | first drawing ms | ranges | bytes | decode work | blocks | peak resident | index bytes | advanced_set_bytes | resident - advanced | held |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mixed | 100000 | wide | 0.2 | 1.9 | 5 | 1627776 | 3255552 | 628 | 2412064 | 392896 | 0 | 2412064 | 1627776 |
| mixed | 100000 | mid | 0.2 | 0.3 | 3 | 261792 | 523584 | 101 | 722080 | 392896 | 0 | 722080 | 261792 |
| mixed | 100000 | tight | 0.2 | 0.0 | 1 | 18144 | 36288 | 7 | 470480 | 392896 | 0 | 470480 | 18144 |
| mixed | 1000000 | wide | 0.8 | 1.3 | 5 | 1910304 | 3525120 | 680 | 7052400 | 3923856 | 0 | 7052400 | 1762560 |
| mixed | 1000000 | mid | 0.8 | 0.2 | 2 | 347328 | 642816 | 124 | 4698208 | 3923856 | 0 | 4698208 | 321408 |
| mixed | 1000000 | tight | 1.6 | 0.0 | 1 | 31104 | 57024 | 11 | 4698208 | 3923856 | 0 | 4698208 | 28512 |
| spline-heavy | 100000 | wide | 0.2 | 11.7 | 6 | 4228880 | 8457760 | 474 | 7524840 | 295688 | 5705056 | 1819784 | 1228608 |
| spline-heavy | 100000 | mid | 0.1 | 3.4 | 4 | 3202448 | 6404896 | 78 | 6498408 | 295688 | 5948704 | 549704 | 202176 |
| spline-heavy | 100000 | tight | 0.1 | 3.1 | 2 | 3015824 | 6031648 | 6 | 6311784 | 295688 | 5957856 | 353928 | 15552 |
| spline-heavy | 1000000 | wide | 1.2 | 85.7 | 8 | 31433648 | 62685856 | 518 | 56286880 | 2943984 | 50978992 | 5307888 | 1342656 |
| spline-heavy | 1000000 | mid | 1.2 | 93.0 | 5 | 30262064 | 60487840 | 94 | 55187872 | 2943984 | 51663072 | 3524800 | 243648 |
| spline-heavy | 1000000 | tight | 1.6 | 81.0 | 4 | 30023600 | 60042016 | 8 | 54964960 | 2943984 | 51440160 | 3524800 | 20736 |
| straddler-heavy | 100000 | wide | 0.1 | 1.2 | 8 | 1760160 | 3369600 | 650 | 2413128 | 391368 | 0 | 2413128 | 1684800 |
| straddler-heavy | 100000 | mid | 0.1 | 0.2 | 7 | 329728 | 575424 | 111 | 720552 | 391368 | 0 | 720552 | 287712 |
| straddler-heavy | 100000 | tight | 0.1 | 0.1 | 5 | 53856 | 57024 | 11 | 467464 | 391368 | 0 | 467464 | 28512 |
| straddler-heavy | 1000000 | wide | 1.0 | 1.5 | 9 | 2032544 | 3628800 | 700 | 7047344 | 3918800 | 0 | 7047344 | 1814400 |
| straddler-heavy | 1000000 | mid | 0.8 | 0.3 | 7 | 412896 | 689472 | 133 | 4688544 | 3918800 | 0 | 4688544 | 344736 |
| straddler-heavy | 1000000 | tight | 32.7 | 0.1 | 7 | 67040 | 72576 | 14 | 4688544 | 3918800 | 0 | 4688544 | 36288 |

Load shed (100 viewport changes in 10 s, 8 ms + 100 MB/s per range, no block cache): units per viewport 74.0, units_planned 7395, units_started 1111, units_superseded 99, units_discarded 99, jobs cancelled 99; shed_rate 1.000, useful_fraction 0.911; final scene is the last viewport: true.
