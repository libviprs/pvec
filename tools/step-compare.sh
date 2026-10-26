#!/usr/bin/env bash
# Runs the PVec against STEP comparison (pvec#61, P8.2) for some corpus entries on one native host, in
# containers only, and collects the raw `key=value` files and the CSV rows.
#
#   tools/step-compare.sh mars    OUTDIR ENTRY...      native x86_64 (MARS), linux/amd64
#   tools/step-compare.sh mercury OUTDIR ENTRY...      native arm64 (Colima VM on the Mac mini), linux/arm64
#
# What runs where, per entry, each step in a process of its own so that a peak resident set is the step's:
#   rust image (libhdf5, no network needed after the build)   vs_step prepare | pvec | part21, and the STEP-HDF
#                                                             spike's corpus-write | corpus-read | corpus-merge
#   tools/step-eval image, pinned by digest, --network none,  tools/step-eval/time_read.py (OCCT reads the
#   read-only root                                            Part 21 file), only up to OCCT_MAX_ENTITIES
# Then `vs_step assemble` writes the rows. The scratch tree is removed from inside a container afterwards, and an
# image or volume this script created is removed too. Nothing runs on a host but ssh, docker and tar.
set -euo pipefail

HOST=${1:?mars|mercury}; OUT=${2:?output directory}; shift 2
[ $# -gt 0 ] || { echo "no entries" >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/.." && pwd)
RUST_IMAGE=${RUST_IMAGE:-pvec-rust:1.99.1}
EVAL_IMAGE=${EVAL_IMAGE:-ghcr.io/libviprs/pvec-step-eval@sha256:957058fc03489500ae9866b795ba4ece8dc43305042c916731908d256c07c0dc}
OCCT_MAX_ENTITIES=${OCCT_MAX_ENTITIES:-100000}
NAME=pvec61-$$

case $HOST in
  mars)    SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR devshell@192.168.0.100); D=docker
           PLATFORM=linux/amd64; PLAT=linux-x86_64; MOUNT="/work/nas-work/$NAME:" ;;
  mercury) SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR mercury); D=/opt/homebrew/bin/docker
           PLATFORM=linux/arm64; PLAT=linux-aarch64; MOUNT="$NAME:" ;;
  *) echo "unknown host $HOST" >&2; exit 2 ;;
esac
dock() { "${SSH[@]}" "$D $*"; }
mkdir -p "$OUT/$PLAT"

cleanup() {
  case $HOST in
    mars) "${SSH[@]}" "mkdir -p /work/nas-work && $D run --rm --platform $PLATFORM -v /work/nas-work:/ws alpine:3.20 sh -c 'rm -rf /ws/$NAME'" >/dev/null 2>&1 || true ;;
    mercury) dock "volume rm -f $NAME" >/dev/null 2>&1 || true
             [ -z "${BUILT_IMAGE:-}" ] || dock "rmi $RUST_IMAGE" >/dev/null 2>&1 || true ;;
  esac
}
trap cleanup EXIT

# --- the tree, and the rust image (mercury has none: it is built there and removed at the end)
if [ "$HOST" = mars ]; then
  "${SSH[@]}" "mkdir -p /work/nas-work/$NAME && chmod g+w /work/nas-work/$NAME"
  (cd "$ROOT" && git ls-files -co --exclude-standard -z | xargs -0 tar cf -) | "${SSH[@]}" "tar -xf - -m -C /work/nas-work/$NAME"
  "${SSH[@]}" "mkdir -p /work/nas-work/$NAME/out && chmod -R g+w /work/nas-work/$NAME"
else
  dock "volume create $NAME" >/dev/null
  (cd "$ROOT" && git ls-files -co --exclude-standard -z | xargs -0 tar cf -) | "${SSH[@]}" "$D run --rm -i --platform $PLATFORM -v $NAME:/work alpine:3.20 sh -c 'tar xf - -C /work && mkdir -p /work/out'"
  if ! dock "image inspect $RUST_IMAGE" >/dev/null 2>&1; then
    BUILT_IMAGE=1
    "${SSH[@]}" "$D build -q --platform $PLATFORM -t $RUST_IMAGE - <<'EOF2'
FROM rust:1.99.1-bookworm
RUN rustup component add clippy rustfmt && apt-get update && apt-get install -y --no-install-recommends libhdf5-dev pkg-config clang && rm -rf /var/lib/apt/lists/*
EOF2" >/dev/null
  fi
fi

rust() { # rust 'sh command'  (mounted at /work, cargo caches in /tmp)
  "${SSH[@]}" "$D run --rm --platform $PLATFORM -v ${MOUNT}/work -w /work -e CARGO_TARGET_DIR=/work/.target -e CARGO_HOME=/work/.cargo -e H5_CHUNK_CACHE_MB=256 $RUST_IMAGE sh -c $(printf '%q' "$1")"
}

ENTRIES=("$@")
echo "== $HOST ($PLAT): $(${SSH[@]} 'uptime')"
rust "uname -m; rustc --version; cargo build --release -q -p pvec-bench --benches 2>&1 | tail -3
      (cd spikes/step-hdf && CARGO_TARGET_DIR=/work/.target-hdf cargo build --release -q 2>&1 | tail -3)
      cat /proc/loadavg"
for E in "${ENTRIES[@]}"; do
  echo "== $E"
  rust "set -x
      B() { cargo bench -q -p pvec-bench --bench vs_step -- \"\$@\"; }
      H=/work/.target-hdf/release/measure
      O=/work/out
      B prepare $E \$O
      B pvec $E \$O
      [ -f \$O/$E.stp ] && B part21 $E \$O || true
      if [ -f \$O/$E.shapes ]; then
        grep '^$E,' crates/pvec-bench/benches/corpus/viewports.csv | cut -d, -f3-6 > \$O/$E.vp
        \$H corpus-write $E \$O/$E.shapes \$O && {
          for c in 'contiguous' 'chunk 65536 + shuffle + deflate 6' 'chunk 4096 + shuffle + deflate 6 + latest'; do
            \$H corpus-read $E \$O \$O/$E.vp \"\$c\" || echo \"corpus-read failed: \$c\" >> \$O/$E.errors
          done
          \$H corpus-merge $E \$O || echo 'corpus-merge failed' >> \$O/$E.errors
        } || echo 'corpus-write failed' >> \$O/$E.errors
      fi" 2>&1 | grep -v '^+ ' || true
  N=$(rust "sed -n 's/^entities=//p' /work/out/$E.prepare.kv" 2>/dev/null || echo 0)
  HAS=$(rust "[ -f /work/out/$E.stp ] && echo yes || echo no" 2>/dev/null)
  if [ "${N:-0}" -le "$OCCT_MAX_ENTITIES" ] && [ "$HAS" = yes ]; then
    "${SSH[@]}" "$D run --rm --platform $PLATFORM --network none --read-only --tmpfs /tmp:rw,size=6g -m 8g -v ${MOUNT}/vol $EVAL_IMAGE sh -c 'cd /vol/tools/step-eval && timeout 3000 python3 time_read.py /vol/out/$E.stp > /vol/out/$E.occt.kv 2> /vol/out/$E.occt.err; echo occt rc=\$?'" 2>&1 | tail -2 || true
  fi
done
echo "== decode artefacts (wasm32-unknown-unknown, size-optimised, gzip -9)"
rust "set -e
      export CARGO_PROFILE_RELEASE_OPT_LEVEL=s CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
      mkdir -p /work/out
      (cd spikes/pvec-wasm-standin && CARGO_TARGET_DIR=/work/.target-wasm cargo build --release --target wasm32-unknown-unknown -q 2>&1 | tail -3)
      (cd spikes/step-hdf-wasm && CARGO_TARGET_DIR=/work/.target-wasm-hdf cargo build --release --target wasm32-unknown-unknown -q 2>&1 | tail -3)
      P=/work/.target-wasm/wasm32-unknown-unknown/release/pvec_wasm_standin.wasm
      H=/work/.target-wasm-hdf/wasm32-unknown-unknown/release/step_hdf_wasm.wasm
      n() { cat \"\$@\" | grep -v '^[[:space:]]*\$' | grep -v '^[[:space:]]*//' | wc -l | tr -d ' '; }
      { echo pvec_wasm_bytes=\$(wc -c < \$P); echo pvec_wasm_gz_bytes=\$(gzip -9 -c \$P | wc -c)
        echo hdf_wasm_bytes=\$(wc -c < \$H); echo hdf_wasm_gz_bytes=\$(gzip -9 -c \$H | wc -c)
        echo pvec_decode_lines=\$(n crates/pvec-container/src/header.rs crates/pvec-container/src/manifest.rs crates/pvec-container/src/directory.rs crates/pvec-container/src/reader.rs crates/pvec-container/src/source.rs crates/pvec-container/src/limits.rs crates/pvec-container/src/compress.rs crates/pvec-container/src/negotiate.rs crates/pvec-container/src/blocks/hot.rs crates/pvec-container/src/blocks/arena.rs crates/pvec-container/src/blocks/advanced.rs crates/pvec-container/src/blocks/aligned.rs crates/pvec-container/src/blocks/mod.rs crates/pvec-stream/src/directory.rs crates/pvec-stream/src/viewport.rs)
        echo part21_reference_parse_lines=\$(awk '/^enum Param/,0' crates/pvec-bench/src/part21.rs | grep -v '^[[:space:]]*\$' | grep -v '^[[:space:]]*//' | wc -l | tr -d ' ')
        echo hdf_reader_lines=\$(awk '/ reading\$/,0' spikes/step-hdf/src/h2.rs | grep -v '^[[:space:]]*\$' | grep -v '^[[:space:]]*//' | wc -l | tr -d ' ')
        echo hdf_wasm_harness_lines=\$(n spikes/step-hdf-wasm/src/lib.rs)
      } > /work/out/artefact.kv
      cat /work/out/artefact.kv"
if [ "$HOST" = mars ]; then
  RE=""
  for e in architectural-1e5 architectural-1e3; do for x in "${ENTRIES[@]}"; do [ "$x" = "$e" ] && RE=$e; done; [ -n "$RE" ] && break; done
  if [ -n "$RE" ]; then
    VP=$(grep "^$RE,0," "$ROOT/crates/pvec-bench/benches/corpus/viewports.csv" | cut -d, -f3-6 | tr ',' ' ')
    "${SSH[@]}" "$D run --rm --platform $PLATFORM --network none -v ${MOUNT}/work -w /work node:22 sh -c 'node spikes/pvec-wasm-standin/run.mjs .target-wasm/wasm32-unknown-unknown/release/pvec_wasm_standin.wasm out/$RE.pvec $VP > out/wasm-run.txt'" || true
    rust "echo pvec_wasm_run=$RE:\$(tr ' ' '_' < /work/out/wasm-run.txt) >> /work/out/artefact.kv" || true
  fi
fi
echo "== assemble"
rust "B() { cargo bench -q -p pvec-bench --bench vs_step -- \"\$@\"; }
      B header > /work/out/rows.csv
      B assemble /work/out $PLAT $HOST ${ENTRIES[*]} >> /work/out/rows.csv
      cd /work/out && { for f in *.kv *.errors *.occt.err; do [ -f \$f ] && { echo \"## \$f\"; cat \$f; }; done; } > raw.txt; true"
"${SSH[@]}" "$D run --rm --platform $PLATFORM -v ${MOUNT}/vol alpine:3.20 sh -c 'cat /vol/out/rows.csv'" > "$OUT/$PLAT/rows.csv"
"${SSH[@]}" "$D run --rm --platform $PLATFORM -v ${MOUNT}/vol alpine:3.20 sh -c 'cat /vol/out/raw.txt'" > "$OUT/$PLAT/raw.txt"
echo "rows: $(wc -l < "$OUT/$PLAT/rows.csv"), raw: $(wc -l < "$OUT/$PLAT/raw.txt") lines in $OUT/$PLAT"
