#!/usr/bin/env bash
# Runs `cargo fmt --all` on MARS in a container and copies the formatted Rust sources back,
# so rustfmt never has to exist on this machine.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
NAS=${NAS:-devshell@192.168.0.100}
WORK=${WORK:-/work/nas-work}
IMAGE=${1:-pvec-rust:1.99.1}
NAME=pvec-fmt-$$
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME"
"$NAS_SH" run "$NAME" "$IMAGE" "export CARGO_HOME=/tmp/cargo; cargo fmt --all; cargo generate-lockfile >/dev/null 2>&1; for d in spikes/*/; do [ -f \$d/Cargo.toml ] && (cd \$d && cargo fmt); done; true"
dirs=$(cd "$ROOT" && ls -d crates tests spikes 2>/dev/null | tr '\n' ' ' || true)
ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS" "cd $WORK/$NAME && tar -cf - $dirs Cargo.lock" | tar -xf - -C "$ROOT"
