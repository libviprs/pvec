#!/usr/bin/env bash
# Builds and tests the workspace with the declared MSRV toolchain on MARS (native x86_64).
# Pulls rust:<msrv>-bookworm if it is missing and untags it again afterwards unless KEEP_IMAGE=1.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
NAS=${NAS:-devshell@192.168.0.100}
MSRV=$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
IMAGE=rust:$MSRV-bookworm
NAME=pvec-msrv-$$
had=$(ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS" "docker images -q $IMAGE")
cleanup() {
  "$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true
  if [ -z "$had" ] && [ "${KEEP_IMAGE:-0}" != 1 ]; then
    ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS" "docker rmi $IMAGE" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
"$NAS_SH" push "$ROOT" "$NAME"
"$NAS_SH" run "$NAME" "$IMAGE" "set -e; export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; rustc --version; uname -m; cargo test --workspace --locked"
