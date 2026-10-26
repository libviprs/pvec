#!/usr/bin/env bash
# Proves the benchmark corpus (P8.1) is obtainable with no proprietary converter image and no network: the
# crates are fetched once, then the whole corpus is rebuilt and checked against its lock in a plain Rust image
# started with --network none. Runs on MARS (native x86_64) through the shared wrapper; nothing but ssh,
# docker and tar runs on this machine, and the scratch tree is removed afterwards.
#
#   tools/corpus-offline.sh [rust-image]
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
NAS=${NAS:-devshell@192.168.0.100}
export NAS
IMAGE=${1:-rust:1.99.1-bookworm}
NAME=${GATE_NAME:-pvec-corpus-$$}
WORK=/work/nas-work

trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME"
"$NAS_SH" run "$NAME" "$IMAGE" "CARGO_HOME=/work/.cargo cargo fetch --locked"
ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS" "docker run --rm --network none --platform linux/amd64 \
  -v $WORK/$NAME:/work -w /work -e CARGO_HOME=/work/.cargo -e CARGO_TARGET_DIR=/tmp/target $IMAGE sh -c \
  'uname -m; cargo test --offline --locked --release -p pvec-bench -- --include-ignored && \
   cargo bench --offline --locked -q -p pvec-bench --bench gen_corpus -- check'"
