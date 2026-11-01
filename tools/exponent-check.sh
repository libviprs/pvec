#!/usr/bin/env bash
# The exponent-number checks on their own (pvec#241): no tolerance-shaped literal such as 1e-9 in code or docs
# outside the numeric policy. The gate and the release script run this first, so a stray literal fails in seconds.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -q -p pvec-checks --test tolerance_literals --test issue_67_docs -- \
  no_tolerance_literal_outside_the_policy_module only_the_numeric_policy_defines_a_tolerance
