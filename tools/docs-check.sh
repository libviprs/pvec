#!/usr/bin/env bash
# The documentation gate (pvec#67): the text checks, rustdoc with warnings denied, and the doctests of the public API.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -q -p pvec-checks
cargo test -q -p pvec-container --test issue_67_walkthrough --test container_doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --lib --no-deps --exclude pvec-bench -q
cargo test --doc -q -p pvec
