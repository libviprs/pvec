# Toolchain decision

Edition **2024**, minimum supported Rust version **1.85**, stable channel only.

I picked 2024 because this is a new crate family with no legacy to carry, and the 2024 edition
tightens a few things that matter for the kind of code PVec is going to contain: `unsafe_op_in_unsafe_fn`
is a warning by default, `extern` blocks have to say `unsafe`, and `static mut` references are
denied. 1.85 is the first release that stabilises the 2024 edition, so it is also the lowest MSRV
the edition allows. Raising it later is cheap; lowering it is not, so nothing newer than 1.85 gets
used until something concrete needs it. In particular `f64::next_up` (1.86) is not used, and the
numeric policy implements its own ULP step from the bit pattern.

Rules that go with it:

- `unsafe` is denied at the workspace level for the library crates and has to be allowed per
  module with a `// SAFETY:` comment on every block. Spikes may use it where a measurement needs it,
  with the same comment rule.
- Every number that comes from outside the process (a file, a Part 21 text, a network range) is
  bounds-checked against the limits in `docs/NUMERIC_POLICY.md` before it sizes an allocation.
- Cargo.lock is committed. Library crates have no dependency that a spike introduced.

The MSRV claim is only as good as the last run that built with it. `tools/msrv.sh` builds and tests
the workspace with the declared MSRV toolchain on MARS (native x86_64). The latest run, on the commit
that introduced this file, used rustc 1.85 and passed all checks. The gate itself runs on 1.99.1.
