#!/usr/bin/env bash
# Runs a command inside spikes/<name> on MARS (native x86_64) in the gate image, then removes the
# scratch tree. The whole repository is pushed because spikes depend on crates/ and tests/.
#
#   tools/spike.sh geometry-api 'cargo test'
#   IMAGE=... tools/spike.sh <name> '<command>'
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
IMAGE=${IMAGE:-pvec-rust:1.99.1}
NAME=pvec-spike-$1-$$
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME" >/dev/null
"$NAS_SH" run "$NAME" "$IMAGE" "export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; uname -m; cd spikes/$1 && $2"
