#!/usr/bin/env bash
# Runs the whole P1.5 measurement on MARS (native x86_64): native tables, per-process memory,
# real wasm32 builds per candidate, node timings and an objdump of the hot loops. Raw output goes to
# docs/spikes/01-5-data/. Images used: pvec-layouts:1.99.1 (build it with the Dockerfile in
# spikes/layouts/Dockerfile) and node:22-bookworm. Scratch is removed at the end.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
NAS=${NAS:-devshell@192.168.0.100}
NAME=pvec-layouts-$$
OUT=$ROOT/docs/spikes/01-5-data
mkdir -p "$OUT"
RUST=pvec-layouts:1.99.1
STAGES=${STAGES:-tables,memory,vec,wasm,node}
SUFFIX=${SUFFIX:-}
has() { case ",$STAGES," in *",$1,"*) return 0;; esac; return 1; }
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT
"$NAS_SH" push "$ROOT" "$NAME" >/dev/null
run() { "$NAS_SH" run "$NAME" "$@"; }
ENVS="export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo; cd spikes/layouts"

if has tables; then
echo "== native tables" >&2
run $RUST "$ENVS; mkdir -p /work/out; uname -m; rustc --version; cargo build --release -q 2>&1 | tail -3; cp /tmp/target/release/measure /work/out/measure; /work/out/measure sizes /work/out; /work/out/measure kinds; ls -l /work/out" > "$OUT/native-tables$SUFFIX.txt"
fi
if has memory; then
echo "== memory" >&2
run $RUST "$ENVS; for f in small-synthetic spline-heavy million straddler; do for c in B A A-raw C base; do /work/out/measure mem \$c /work/out/\$f.\$c.bin; /work/out/measure mem \$c /work/out/\$f.\$c.bin mat; done; done" > "$OUT/memory$SUFFIX.txt"
fi
if has vec; then
echo "== vectorisation (objdump of the native hot loops)" >&2
run $RUST "cd spikes/layouts; ./count_simd.sh /work/out/measure" > "$OUT/vectorisation$SUFFIX.txt"
fi
if has wasm; then
echo "== wasm builds" >&2
run $RUST "$ENVS; mkdir -p /work/out/wasm; for c in b a c base; do cargo build --release --lib --target wasm32-unknown-unknown --no-default-features --features cand-\$c --config 'profile.release.strip=\"symbols\"' 2>&1 | tail -2; cp /tmp/target/wasm32-unknown-unknown/release/layouts_spike.wasm /work/out/wasm/\$c.wasm; done; ls -l /work/out/wasm; for c in a b c base; do echo \"deps \$c: \$(cargo tree -e normal --no-default-features --features cand-\$c --prefix none | sort -u | grep -vc layouts-spike) (excluding the spike and pvec-fixtures)\"; done; cargo tree -e normal --no-default-features --features cand-a --prefix none | sort -u; cargo tree -e normal --no-default-features --features cand-c --prefix none | sort -u" > "$OUT/wasm-build$SUFFIX.txt" 2>&1
fi
if has node; then
echo "== wasm timings (node 22)" >&2
JS=/work/spikes/layouts/js/bench.mjs
N_small=1000; N_spline=50000; N_million=1000000; N_strad=100000
CMDS=""
for f in small-synthetic:1000 spline-heavy:50000 million:1000000 straddler:100000; do
  fx=${f%%:*}; n=${f##*:}
  CMDS="$CMDS node $JS /work/out/wasm/b.wasm touch_b /work/out/$fx.B.bin $n;"
  CMDS="$CMDS node $JS /work/out/wasm/a.wasm touch_a /work/out/$fx.A.bin $n;"
  CMDS="$CMDS node $JS /work/out/wasm/a.wasm touch_a_raw /work/out/$fx.A-raw.bin $n;"
  CMDS="$CMDS node $JS /work/out/wasm/c.wasm touch_c /work/out/$fx.C.bin $n;"
  CMDS="$CMDS node $JS /work/out/wasm/base.wasm touch_base /work/out/$fx.base.bin $n;"
done
"$NAS_SH" run "$NAME" node:22-bookworm "node --version; $CMDS" > "$OUT/wasm-timings$SUFFIX.txt"
echo done >&2
fi
