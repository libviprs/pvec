#!/usr/bin/env bash
# Coverage-guided fuzzing of the PVec reader (issue P7.8), in throw-away containers on the fleet hosts.
#
#   tools/fuzz.sh [--host mars|nas|mercury|jupiter] [--seconds N] [--no-checksum-gate]
#                 [--keep-corpus] [TARGET ...]
#
# Budget: 300 seconds per target is the smoke gate of a pull request, 14400 the nightly. The default host is
# mercury, because arm64 is the default and an x86_64 run goes to mars or nas (native x86_64, never Rosetta).
# Every `docker run` carries --platform. Nothing runs on a host but ssh and docker: the toolchain, cargo-fuzz and
# the build are in the image, the sources and the corpus are in named volumes.
#
# The corpus persists between runs in the volume pvec-fuzz-corpus on the host. The seeds (`seed-corpus`) are
# added to it without overwriting what a run has grown. Crashes come back to .fuzz-out/<run>/<host>/<target>/
# and are never deleted from the host's volume until the volumes are removed at the end of the run.
#
# Parallel runs: build the image once first (a run that finds it missing builds it and would race on the tag), give each
# invocation its own RUN_ID and its own CORPUS_VOL, and set FUZZ_CPUS to cap a container's cpus.
#
# What it removes at the end: the volumes it created and, if it built it, the image. No docker prune.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
HOST=mercury
SECONDS_PER_TARGET=300
FEATURES=""
KEEP_CORPUS=0
TARGETS=()
while [ $# -gt 0 ]; do
  case $1 in
    --host) HOST=$2; shift 2 ;;
    --seconds) SECONDS_PER_TARGET=$2; shift 2 ;;
    --no-checksum-gate) FEATURES="--features no-checksum-gate"; shift ;;
    --keep-corpus) KEEP_CORPUS=1; shift ;;
    -h|--help) sed -n 2,19p "$0"; exit 0 ;;
    -*) echo "fuzz.sh: unknown option $1" >&2; exit 64 ;;
    *) TARGETS+=("$1"); shift ;;
  esac
done
[ ${#TARGETS[@]} -gt 0 ] || TARGETS=(fuzz_header fuzz_directory_traversal fuzz_immediate_blocks fuzz_advanced_blocks fuzz_decompress_boundary fuzz_component_streams)

case $HOST in
  # ~/.docker on mars is a dangling symlink, which makes `docker build` fail with "mkdir: file exists", so
  # the client config goes to the scratch directory the fleet already uses
  mars)    SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR devshell@192.168.0.100); DOCKER=docker; PLATFORM=linux/amd64; MEM=6g; DCFG=/work/nas-work/.docker-config-fuzz ;;
  nas)     SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR rom@192.168.0.10);       DOCKER=docker; PLATFORM=linux/amd64; MEM=6g; DCFG='~/workspace/nas-work/.docker-config-fuzz' ;;
  mercury) SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR mercury);                DOCKER=/opt/homebrew/bin/docker; PLATFORM=linux/arm64; MEM=5g ;;
  jupiter) SSH=();                                                              DOCKER=docker; PLATFORM=linux/arm64; MEM=4g ;;
  *) echo "fuzz.sh: unknown host $HOST" >&2; exit 64 ;;
esac

RUN=${RUN_ID:-fz$(date +%m%d%H%M%S)-$$}
SRC_VOL=pvec-fuzz-src-$RUN
OUT_VOL=pvec-fuzz-out-$RUN
CORPUS_VOL=${CORPUS_VOL:-pvec-fuzz-corpus}
IMAGE=pvec-fuzz-nightly:1
OUT=$ROOT/.fuzz-out/$RUN/$HOST
mkdir -p "$OUT"

# Runs docker on the host; arguments are quoted for the remote shell.
d() {
  if [ ${#SSH[@]} -eq 0 ]; then "$DOCKER" "$@"; else
    local q; q=$(printf '%q ' "$DOCKER" "$@"); "${SSH[@]}" "${DCFG:+DOCKER_CONFIG=$DCFG }$q"
  fi
}

BUILT_IMAGE=0
if ! d image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "[$HOST] building $IMAGE ($PLATFORM)"
  d build --platform "$PLATFORM" -t "$IMAGE" - <<'DOCKERFILE' >"$OUT/image-build.log" 2>&1
FROM rust:1.99.0-bookworm
RUN rustup toolchain install nightly --profile minimal --component rust-src \
 && cargo install cargo-fuzz --locked
DOCKERFILE
  BUILT_IMAGE=1
fi

cleanup() {
  d volume rm "$SRC_VOL" "$OUT_VOL" >/dev/null 2>&1 || true
  [ "$KEEP_CORPUS" = 1 ] || d volume rm "$CORPUS_VOL" >/dev/null 2>&1 || true
  [ "$BUILT_IMAGE" = 0 ] || d image rm "$IMAGE" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# the sources: the working tree without build output, as a tar stream into a named volume
d volume create "$SRC_VOL" >/dev/null
d volume create "$OUT_VOL" >/dev/null
d volume create "$CORPUS_VOL" >/dev/null
tar -C "$ROOT" --exclude=target --exclude=.git --exclude=.fuzz-out --exclude=fuzz/target --exclude=fuzz/corpus --exclude=fuzz/artifacts -cf - . |
  d run --rm -i --platform "$PLATFORM" -v "$SRC_VOL":/src "$IMAGE" tar -xf - -C /src

for T in "${TARGETS[@]}"; do
  echo "[$HOST] $T for ${SECONDS_PER_TARGET}s ($PLATFORM${FEATURES:+, no-checksum-gate})"
  d run --rm --platform "$PLATFORM" --memory "$MEM" ${FUZZ_CPUS:+--cpus "$FUZZ_CPUS"} \
    -v "$SRC_VOL":/src -v "$OUT_VOL":/out -v "$CORPUS_VOL":/corpus "$IMAGE" sh -c "
      set -e
      export CARGO_TARGET_DIR=/out/target CARGO_HOME=/out/cargo
      cd /src
      cargo run --release -q -p pvec-fuzz --bin seed-corpus -- /out/seed > /out/seed.txt
      mkdir -p /corpus/$T /out/artifacts/$T
      cp -n /out/seed/$T/* /corpus/$T/ 2>/dev/null || true
      echo corpus-before \$(ls /corpus/$T | wc -l)
      cd fuzz
      cargo +nightly fuzz run $FEATURES $T /corpus/$T -- \
        -max_total_time=$SECONDS_PER_TARGET -rss_limit_mb=2560 -timeout=10 -print_final_stats=1 \
        -artifact_prefix=/out/artifacts/$T/ 2>&1 | tail -n 40
      echo corpus-after \$(ls /corpus/$T | wc -l)
    " 2>&1 | tee "$OUT/$T.log" | grep -E "^\[|corpus-|stat::|ERROR|crash|oom|timeout|SUMMARY" || true
done

# crashes, oom and timeout reproducers come back
d run --rm --platform "$PLATFORM" -v "$OUT_VOL":/out "$IMAGE" sh -c 'cd /out && cat seed.txt; tar -cf - artifacts' | tar -C "$OUT" -xf - 2>/dev/null || true
echo "[$HOST] logs and artifacts in $OUT"
