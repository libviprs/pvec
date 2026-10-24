#!/usr/bin/env bash
# The GeoPackage export checked by programs that did not write it: the GeoPackage validator that
# ships with GDAL (validate_gpkg.py), SQLite's own integrity and R-tree checks, and GDAL's OGR as the
# independent GIS reader. The samples are written inside a Rust container, then checked inside a
# throw-away GDAL container with no network, a read-only root and the samples mounted read-only.
# Nothing but ssh, docker and tar runs on this machine, and the scratch tree is removed afterwards.
#
#   tools/gpkg-validate.sh [host]        host: ssh target, default MARS (devshell@192.168.0.100)
#
# The GDAL image is pinned by digest. The first run pulls it (that is the only network use).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS=${1:-${NAS:-devshell@192.168.0.100}}
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
RUST_IMAGE=${RUST_IMAGE:-pvec-rust:1.99.1}
# ghcr.io/osgeo/gdal:ubuntu-small-3.9.3, GDAL 3.9.3 of 2024-10-07, SQLite 3.45.1
GDAL_IMAGE=${GDAL_IMAGE:-ghcr.io/osgeo/gdal@sha256:7125d6f5d6af5331354328fc6a64c9e420b29b31460d1a6aaf9526e559255e07}
NAME=${GATE_NAME:-pvec-gpkg-$$}
case $NAS in devshell@*) WORK=/work/nas-work ;; *) WORK='$HOME/workspace/nas-work' ;; esac
SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS")
PLATFORM=$(case $("${SSH[@]}" uname -m) in x86_64) echo linux/amd64 ;; *) echo linux/arm64 ;; esac)

trap 'NAS=$NAS "$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
NAS=$NAS "$NAS_SH" push "$ROOT" "$NAME"
echo "== writing the samples ($RUST_IMAGE, $PLATFORM)"
NAS=$NAS "$NAS_SH" run "$NAME" "$RUST_IMAGE" "
set -e
export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo
mkdir -p /work/gpkg-out
cargo run -q --locked -p pvec-interchange --example gpkg_samples -- /work/gpkg-out
chmod -R a+rX /work/gpkg-out
"
echo "== checking them ($GDAL_IMAGE)"
"${SSH[@]}" "docker pull -q --platform $PLATFORM $GDAL_IMAGE >/dev/null && docker run --rm --platform $PLATFORM \
  --network none --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges \
  -v $WORK/$NAME/gpkg-out:/samples:ro -v $WORK/$NAME/tools/gpkg-validate:/check:ro \
  --entrypoint python3 $GDAL_IMAGE -I /check/check.py /samples"
