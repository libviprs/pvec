#!/usr/bin/env bash
# Runs the layout, index and compression benchmark (pvec#62, P8.3) on one native host, in containers only. Per
# entry: the shipped half (`benches/layout.rs`: decode throughput, the shipped index and block size), the index
# models and the block size sweep (`spikes/spatial-index corpus-index`) and the codecs (`spikes/codec-sweep`), all
# on the same files, all writing tidy rows to OUT/<platform>/layout.rows.csv.
#
#   tools/layout-bench.sh mars|mercury OUTDIR [ENTRY...]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/tools/bench-host.sh" "${1:?mars|mercury}" "pvec62-$$"
OUT=${2:?output directory}; shift 2
ENTRIES=("$@")
[ ${#ENTRIES[@]} -gt 0 ] || ENTRIES=(pathological-1e3 architectural-1e3 architectural-1e5 mep-1e5 industrial-1e5 huge-coordinate-1e5 spline-heavy-1e5 architectural-1e6 spline-heavy-1e6)
mkdir -p "$OUT/$PLAT"
trap host_cleanup EXIT
host_push "$ROOT"
echo "== $BH_HOST ($PLAT): $("${SSH[@]}" uptime)"
rust "uname -m; rustc --version
      cargo build --release -q -p pvec-bench --benches 2>&1 | tail -3
      (cd spikes/spatial-index && CARGO_TARGET_DIR=/work/.target-si cargo build --release -q 2>&1 | tail -3)
      (cd spikes/codec-sweep && CARGO_TARGET_DIR=/work/.target-cs cargo build --release -q 2>&1 | tail -3)
      cargo bench -q -p pvec-bench --bench layout -- header > /work/out/layout.rows.csv; cat /proc/loadavg"
for E in "${ENTRIES[@]}"; do
  echo "== $E"
  rust "O=/work/out
        L() { cargo bench -q -p pvec-bench --bench layout -- \"\$@\"; }
        L prepare $E \$O
        grep '^$E,' crates/pvec-bench/benches/corpus/viewports.csv | cut -d, -f3-6 > \$O/$E.vp
        { L decode $E \$O $PLAT; L shipped $E \$O $PLAT
          /work/.target-si/release/measure corpus-index $E \$O/$E.shapes \$O/$E.vp $PLAT
          /work/.target-cs/release/measure $E \$O/$E.pvec $PLAT; } >> \$O/layout.rows.csv; cat /proc/loadavg" 2>&1 | tail -2
done
fetch out/layout.rows.csv > "$OUT/$PLAT/layout.rows.csv"
echo "rows: $(wc -l < "$OUT/$PLAT/layout.rows.csv") in $OUT/$PLAT"
