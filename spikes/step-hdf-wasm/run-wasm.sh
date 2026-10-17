#!/usr/bin/env bash
# Orchestrates P1.4's wasm measurement on MARS: write the files with libhdf5, build the wasm reader,
# run it under node. Three containers (hdf5 image, rust image, node image) on one scratch tree.
#   run-wasm.sh            (from the repository root)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
NAME=pvec-wasm-$$
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME" >/dev/null
echo "load average: $(ssh -o BatchMode=yes -o LogLevel=ERROR "${NAS:-devshell@192.168.0.100}" cat /proc/loadavg)"
echo "== write the files (libhdf5 1.10.8, native x64)"
"$NAS_SH" run "$NAME" pvec-hdf5:1.99.1 "export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; cd spikes/step-hdf && cargo build --release -q && mkdir -p /work/out && for f in small-synthetic spline-heavy million; do for l in h1 h2; do /tmp/target/release/measure sizes \$f \$l contiguous /work/out >/dev/null; if [ \$l = h1 ]; then /tmp/target/release/measure sizes \$f h1 'contiguous+latest' /work/out >/dev/null; fi; /tmp/target/release/measure sizes \$f \$l 'chunk 65536 + shuffle + deflate 6' /work/out > /dev/null; done; done; ls -la /work/out; for f in /work/out/*h2-contiguous.h5 /work/out/*h2-chunk*.h5; do echo \$(basename \$f) native hash \$(/tmp/target/release/measure hash h2 \$f); done; for f in /work/out/*h1-contiguousplatest.h5; do echo \$(basename \$f) native hash \$(/tmp/target/release/measure hash h1 \$f); done"
echo "== build the wasm reader (rustc 1.99.1, wasm32-unknown-unknown)"
"$NAS_SH" run "$NAME" pvec-rust:1.99.1 "export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; cd spikes/step-hdf-wasm && cargo build --release --target wasm32-unknown-unknown 2>&1 | tail -2 && mkdir -p /work/out && cp /tmp/target/wasm32-unknown-unknown/release/step_hdf_wasm.wasm /work/out/default.wasm && CARGO_PROFILE_RELEASE_OPT_LEVEL=s CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo build --release --target wasm32-unknown-unknown --target-dir /tmp/target-s 2>&1 | tail -1 && cp /tmp/target-s/wasm32-unknown-unknown/release/step_hdf_wasm.wasm /work/out/size-optimised.wasm && ls -la /work/out/*.wasm && for w in default size-optimised; do echo \$w gzip \$(gzip -9 -c /work/out/\$w.wasm | wc -c) bytes; done"
echo "== run under node 22 (V8)"
"$NAS_SH" run "$NAME" node:22 "node --version; for w in default size-optimised; do node spikes/step-hdf-wasm/harness.js /work/out/\$w.wasm h2 /work/out/small-synthetic-h2-contiguous.h5 /work/out/spline-heavy-h2-contiguous.h5 /work/out/million-h2-contiguous.h5 /work/out/spline-heavy-h2-chunk_65536_p_shuffle_p_deflate_6.h5 /work/out/million-h2-chunk_65536_p_shuffle_p_deflate_6.h5; done; node spikes/step-hdf-wasm/harness.js /work/out/size-optimised.wasm h1 /work/out/small-synthetic-h1-contiguousplatest.h5 /work/out/spline-heavy-h1-contiguousplatest.h5 /work/out/million-h1-contiguousplatest.h5"
