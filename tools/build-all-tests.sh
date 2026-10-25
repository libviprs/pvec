#!/usr/bin/env bash
# Guard: every test target in the workspace has to compile, so two PRs that merge separately and
# only break each other (pvec#160) show up as red here before anything runs.
#
#   tools/build-all-tests.sh
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --workspace --locked --no-run
