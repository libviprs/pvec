#!/usr/bin/env bash
# Re-derives tests/corpus/ from a checkout of libviprs/pvec-gold-fixtures at the pinned tag and checks
# that the vendored files are byte-identical to what the tag produces. Maintenance only: the test
# suite never runs this and never touches the network.
#
#   tools/corpus-vendor.sh [path-to-pvec-gold-fixtures]      (default ../pvec-gold-fixtures)
#   WRITE=1 tools/corpus-vendor.sh ...                        also overwrites tests/corpus/ when it differs
#
# The corpus's own bundler runs in a throw-away python container; nothing else executes here.
set -euo pipefail

# The tag corpus-r0-open still points at f94f7f22, before the transform records moved their structural
# assertion to SOURCE_MAPPING (#200). The pin is the commit, and the bundle hash in the pin file is the proof.
TAG=corpus-r0-open
COMMIT=1df9d1b3e75047fa3978ebd809d0128cdfb28a68
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SRC=${1:-$ROOT/../pvec-gold-fixtures}
SCRATCH=${SCRATCH:-$(mktemp -d)}

git -C "$SRC" cat-file -e "$COMMIT^{commit}" || { echo "FAIL: $COMMIT is not in $SRC (fetch it)" >&2; exit 1; }
mkdir -p "$SCRATCH/src" "$SCRATCH/out"
git -C "$SRC" archive "$COMMIT" | tar -x -C "$SCRATCH/src"
docker run --rm --platform linux/arm64 -e PYTHONDONTWRITEBYTECODE=1 \
  -v "$SCRATCH/src:/g:ro" -v "$SCRATCH/out:/o" -w /g python:3.12-slim \
  python3 -m runner.r0_open bundle corpus/r0-open /o/r0-open.tar

want=$(sed -n 's/.*"bundle_sha256": "\([0-9a-f]*\)".*/\1/p' "$SCRATCH/src/corpus/r0-open.pin.json")
have=$(shasum -a 256 "$SCRATCH/out/r0-open.tar" | cut -d' ' -f1)
if [ "$want" != "$have" ]; then
  echo "FAIL: the tag's pin says $want but its bundle hashes to $have" >&2
  exit 1
fi
status=0
for f in r0-open.tar:out/r0-open.tar r0-open.pin.json:src/corpus/r0-open.pin.json; do
  name=${f%%:*}
  if ! cmp -s "$ROOT/tests/corpus/$name" "$SCRATCH/${f#*:}"; then
    status=1
    echo "DIFFERS: tests/corpus/$name"
    if [ "${WRITE:-0}" = 1 ]; then cp "$SCRATCH/${f#*:}" "$ROOT/tests/corpus/$name"; fi
  fi
done
echo "scratch left in $SCRATCH (remove it when you are done)"
[ $status = 0 ] && echo "OK: tests/corpus matches $COMMIT, bundle sha256 $have"
exit $status
