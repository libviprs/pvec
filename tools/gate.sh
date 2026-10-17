#!/usr/bin/env bash
# Local gate: fmt, clippy, the full test suite and the wasm32 build of the geometry crate, run on
# MARS (native x86_64) in a throw-away container through the shared wrapper. Nothing but ssh,
# docker and tar runs on this machine.
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
echo '== the geometry crate builds for wasm32 (pure Rust, no file or network access)'
cargo build -p pvec-geometry --target wasm32-unknown-unknown --locked
"
