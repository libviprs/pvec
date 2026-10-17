#!/usr/bin/env bash
# Orchestrates P1.8's compression and checksum measurements on MARS (native x86_64): native tables, a
# wasm32 build, and the same decode and checksum work under node 22. From the repository root.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
NAME=pvec-cc-$$
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME" >/dev/null
echo "== native (rustc, x86_64)"
"$NAS_SH" run "$NAME" pvec-rust:1.99.1 "export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; cd spikes/compress-checksum && cargo build --release -q --bin measure && for f in small-synthetic spline-heavy million straddler; do /tmp/target/release/measure compress \$f; /tmp/target/release/measure checksum \$f; done; mkdir -p /work/out && for f in spline-heavy million; do /tmp/target/release/measure dump \$f /work/out; done"
echo "== wasm32 build"
"$NAS_SH" run "$NAME" pvec-rust:1.99.1 "export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; cd spikes/compress-checksum && cargo build --release --lib --target wasm32-unknown-unknown 2>&1 | tail -2 && cp /tmp/target/wasm32-unknown-unknown/release/compress_checksum_spike.wasm /work/out/cc.wasm && ls -la /work/out/cc.wasm"
echo "== node 22"
"$NAS_SH" run "$NAME" node:22 "node --version; for f in spline-heavy million; do node spikes/compress-checksum/harness.js /work/out/cc.wasm /work/out \$f; done"
