#!/usr/bin/env bash
# Local gate: fmt, clippy and the full test suite, run on MARS (native x86_64) in a throw-away
# container through the shared wrapper. Nothing but ssh, docker and tar runs on this machine.
#
#   tools/gate.sh [rust-image] [extra cargo test args...]
#
# The scratch tree is removed afterwards. Set NAS_SH to point at a different wrapper.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
IMAGE=${1:-pvec-rust:1.99.1}
shift || true
NAME=${GATE_NAME:-pvec-gate-$$}

trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME"
"$NAS_SH" run "$NAME" "$IMAGE" "
set -e
export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo
rustc --version; uname -m
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked $*
for d in spikes/*/; do
  echo == spike \$d
  (cd \$d && cargo fmt -- --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked)
done
echo '== red suite: the evaluator probes must still fail until P3.1 lands'
if cargo test -p pvec-numeric --test pathological --locked -- --ignored >/tmp/red.log 2>&1; then
  echo 'FAIL: the evaluator probes went green; update tests/pathological/main.rs and this step'; exit 1
fi
grep -E 'no evaluator exists yet|test result' /tmp/red.log
"
