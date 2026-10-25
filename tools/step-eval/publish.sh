#!/usr/bin/env bash
# Builds the STEP evaluator image on a native host of each architecture, pushes both to ghcr.io/libviprs and joins
# them in one multi-platform index, then prints the index digest to pin in crates/pvec-interchange/tests/step_roundtrip.rs.
# Images are built on mars (amd64) and mercury (arm64), never on the MacBook.
#
#   tools/step-eval/publish.sh [tag]        tag defaults to bookworm-freecad-<version>
#
# The registry token comes from `gh auth token` over stdin into a throw-away container that has the push tool and
# keeps its login on its own filesystem. It is never written to a host's disk. The package has to be public for a
# lane without a login to pull it (GitHub has no API for that: package settings, change visibility).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=ghcr.io/libviprs/pvec-step-eval
TAG=${1:-bookworm-freecad-0.20.2}
CRANE=gcr.io/go-containerregistry/crane:debug
MARS=devshell@192.168.0.100
MERCURY=mercury
MD=/opt/homebrew/bin/docker

build_push() { # build_push SSH DOCKER PLATFORM ARCHTAG
  tar -C "$HERE" -cf - Dockerfile | ssh -o BatchMode=yes "$1" 'd=$(mktemp -d "${TMPDIR:-/tmp}/step-eval.XXXXXX") && tar -C "$d" -xf - && echo "$d"' >/tmp/step-eval.dir.$$
  local d; d=$(cat /tmp/step-eval.dir.$$); rm -f /tmp/step-eval.dir.$$
  gh auth token | ssh -o BatchMode=yes "$1" "set -e; read -r GHT; export GHT; T=\$HOME/step-eval-$4.tar
    $2 build -q --platform $3 -t pvec-step-eval:$4 $d && $2 save pvec-step-eval:$4 -o \$T
    $2 run --rm -e GHT -v \$T:/i.tar:ro --entrypoint sh $CRANE -c 'echo \$GHT | crane auth login ghcr.io -u spdrman --password-stdin >/dev/null 2>&1 && crane push /i.tar $REPO:$4'
    rm -f \$T"
}
build_push $MARS docker linux/amd64 amd64
build_push $MERCURY $MD linux/arm64 arm64
gh auth token | ssh -o BatchMode=yes $MARS "read -r GHT; export GHT; docker run --rm -e GHT --entrypoint sh $CRANE -c 'echo \$GHT | crane auth login ghcr.io -u spdrman --password-stdin >/dev/null 2>&1 && crane index append -t $REPO:$TAG -m $REPO:amd64 -m $REPO:arm64 >/dev/null && crane digest $REPO:$TAG'"
