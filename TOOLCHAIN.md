# Toolchain pins

libviprs, planviewer and the server all build against this repository by git rev. They have to build against the
same one, or a file one of them writes is read by code that disagrees with it.

## Pin check

```
tools/pin-check.sh <checkout> [<checkout> ...]
```

For each checkout path it reads the pvec rev of every dependency on `https://github.com/libviprs/pvec` in
`Cargo.toml` and in `tests/pdf-compare/gen/Cargo.toml` (planviewer's PDF compare generator), and compares them with
`pvec.pin.json` when there is one. It prints one line per dependency, then a verdict.

| Exit | Meaning |
| ---- | ------- |
| 0 | every rev is the same |
| 1 | two or more revs, a pvec dependency without a `rev`, or a `pvec.pin.json` whose `commit` or `crates` list differs from `Cargo.toml` |
| 2 | bad arguments, or no checkout has a pvec dependency |

A checkout with no pvec dependency yet (the server, until it adopts the crates) is reported and skipped. Every crate
counts, not only the ones `pvec.pin.json` names. The check reads files only and runs no cargo, so it runs anywhere.

The check never edits a pin. A mismatch is fixed in the repository that holds it.

## Test

`crates/pvec-checks/tests/issue_304_pin_check.rs` runs the script over the fixtures in `tests/pin-check/`: one set
with equal revs (passes), one with a split rev in the generator (fails), and small cases for a missing rev, a short
`pvec.pin.json` and a checkout without pvec.
