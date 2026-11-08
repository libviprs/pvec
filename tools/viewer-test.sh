#!/usr/bin/env bash
# Builds the loadable pvec-wasm module into viewer/ and runs the viewer's browser tests (ADR 0020), both in
# throw-away containers on the docker host. Nothing runs here but ssh and tar.
#
#   tools/viewer-test.sh [playwright args...]   e.g. tools/viewer-test.sh -g '#271'
#   tools/viewer-test.sh --clean                remove the cache volumes from the docker host
#
# NAS picks the host the way ../tools/nas.sh does (MARS by default; NAS=rom@192.168.0.10 for the NAS). The
# cargo registry, the wasm-bindgen CLI and the target directory live in two named volumes on that host so a
# second run does not rebuild them; --clean removes them. PLAYWRIGHT_IMAGE must match the version in
# viewer/tests/package.json.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS=${NAS:-devshell@192.168.0.100}
export NAS
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
RUST_IMAGE=${RUST_IMAGE:-rust:1.99.1-bookworm}
PLAYWRIGHT_IMAGE=${PLAYWRIGHT_IMAGE:-mcr.microsoft.com/playwright:v1.50.1-noble}
PLATFORM=${PLATFORM:-linux/amd64}
SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS")
CACHE=(-v pvec-viewer-cargo:/opt/cargo -v pvec-viewer-target:/opt/target)

if [ "${1:-}" = "--clean" ]; then
  "${SSH[@]}" "docker volume rm pvec-viewer-cargo pvec-viewer-target" || true
  exit 0
fi

case $NAS in
  devshell@*) WORK=/work/nas-work ;;
  *) WORK='$HOME/workspace/nas-work' ;;
esac
NAME=pvec-viewer-test-$$
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME" >/dev/null

"${SSH[@]}" "docker run --rm --platform $PLATFORM ${CACHE[*]} -v $WORK/$NAME:/work -w /work $RUST_IMAGE sh tools/viewer-module.sh"

ARGS=$(printf '%q ' "$@")
"${SSH[@]}" "docker run --rm --platform $PLATFORM --ipc=host -v $WORK/$NAME:/work -w /work/viewer/tests $PLAYWRIGHT_IMAGE sh -c '
set -e
npm ci --no-audit --no-fund --loglevel=error >/dev/null
echo browser host: \$(uname -m)
npx playwright test $ARGS
'"
