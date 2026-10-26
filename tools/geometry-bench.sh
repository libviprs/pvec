#!/usr/bin/env bash
# Runs the geometry benchmark (pvec#63, P8.4) on one native host, in containers only: the P1.3 prototype on the
# reference cubic first, then `benches/geometry.rs`, whose rows land in OUT/<platform>/geometry.rows.csv.
#
#   tools/geometry-bench.sh mars|mercury OUTDIR
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/tools/bench-host.sh" "${1:?mars|mercury}" "pvec63-$$"
OUT=${2:?output directory}
mkdir -p "$OUT/$PLAT"
trap host_cleanup EXIT
host_push "$ROOT"
echo "== $BH_HOST ($PLAT): $("${SSH[@]}" uptime)"
rust "uname -m; rustc --version
      (cd spikes/geometry-api && CARGO_TARGET_DIR=/work/.target-proto cargo build --release -q --bin corpus_eval 2>&1 | tail -3)
      /work/.target-proto/release/corpus_eval > /work/out/prototype.kv; cat /work/out/prototype.kv
      cargo bench -q -p pvec-bench --bench geometry -- header > /work/out/geometry.rows.csv
      cargo bench -q -p pvec-bench --bench geometry -- run $PLAT $BH_HOST /work/out/prototype.kv >> /work/out/geometry.rows.csv
      cat /proc/loadavg"
fetch out/geometry.rows.csv > "$OUT/$PLAT/geometry.rows.csv"
fetch out/prototype.kv > "$OUT/$PLAT/prototype.kv"
echo "rows: $(wc -l < "$OUT/$PLAT/geometry.rows.csv") in $OUT/$PLAT"
