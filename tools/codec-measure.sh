#!/usr/bin/env bash
# The P4.6 codec measurement on each platform, in containers only (#110).
#
#   tools/codec-measure.sh x64      native x86_64 on MARS
#   tools/codec-measure.sh wasm     wasm32-wasip1 on MARS, run under node 22's WASI in a second container
#   tools/codec-measure.sh arm64    native arm64 on mercury (Colima), sources in a named volume
#
# Each run prints the markdown tables the test emits, between MEASUREMENT-BEGIN and MEASUREMENT-END.
# The scratch tree (MARS) or volume (mercury) is removed afterwards, from inside a container.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
MARS=${NAS:-devshell@192.168.0.100}
IMAGE=${IMAGE:-pvec-rust:1.99.1}
NODE_IMAGE=${NODE_IMAGE:-node:22-bookworm-slim}
TEST="cargo test --release --locked -p pvec-testkit --test p4_6_codec_measurement"
ARGS="-- --ignored --nocapture print_the_codec_measurement"
NAME=pvec-codec-$1-$$

case ${1:-} in
  x64)
    trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
    "$NAS_SH" push "$ROOT" "$NAME" >/dev/null
    "$NAS_SH" run "$NAME" "$IMAGE" "export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; uname -m; rustc --version; $TEST $ARGS"
    ;;
  wasm)
    trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
    "$NAS_SH" push "$ROOT" "$NAME" >/dev/null
    # Compile for wasm32-wasip1 (std::time works there, wasm32-unknown-unknown has no clock), copy the
    # test binary out of the throw-away target dir into the scratch tree, then run it under node.
    "$NAS_SH" run "$NAME" "$IMAGE" "set -e; export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo
      rustup target add wasm32-wasip1 >/dev/null 2>&1
      rustc --version
      cargo test --release --locked -p pvec-testkit --test p4_6_codec_measurement --target wasm32-wasip1 --no-run
      mkdir -p /work/.wasm-out
      cp \$(ls -t /tmp/target/wasm32-wasip1/release/deps/p4_6_codec_measurement-*.wasm | head -1) /work/.wasm-out/measure.wasm"
    ssh -o BatchMode=yes -o LogLevel=ERROR "$MARS" "docker run --rm --platform linux/amd64 \
      -v /work/nas-work/$NAME:/work -w /work $NODE_IMAGE sh -c 'node --version; uname -m; \
      node --experimental-wasm-modules tools/wasi-run.mjs /work/.wasm-out/measure.wasm --ignored --nocapture print_the_codec_measurement'"
    ;;
  arm64)
    # mercury: docker is not on PATH over ssh, always --platform, one job, sources in a named volume.
    D=/opt/homebrew/bin/docker
    V=pvec-codec-$$
    SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR mercury)
    trap '"${SSH[@]}" "$D run --rm --platform linux/arm64 -v $V:/v alpine:3.20 sh -c \"rm -rf /v/src\"; $D volume rm $V" >/dev/null 2>&1 || true' EXIT
    "${SSH[@]}" "$D volume create $V >/dev/null"
    (cd "$ROOT" && tar --exclude='.git' --exclude='target' -cf - .) | "${SSH[@]}" \
      "$D run --rm -i --platform linux/arm64 -v $V:/v alpine:3.20 sh -c 'mkdir -p /v/src && tar -xf - -C /v/src'"
    "${SSH[@]}" "$D run --rm --platform linux/arm64 -v $V:/work -w /work/src rust:1.99.1-bookworm sh -c 'set -e
      export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; uname -m; rustc --version; $TEST $ARGS'"
    ;;
  *) sed -n '2,/^set -euo/p' "$0" | sed '$d' >&2; exit 2 ;;
esac
