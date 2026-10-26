#!/usr/bin/env bash
# Runs the progressive coverage benchmark (pvec#64, P8.5) on one native host, in containers only. Per entry:
# `prepare` writes the corpus entry with a Tier 0 block per leaf, `run` measures the four stages on the three
# network arms plus the interaction arm and writes tidy rows to OUT/<platform>/progressive.rows.csv.
#
#   tools/progressive-bench.sh mars|mercury OUTDIR [ENTRY...]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
. "$ROOT/tools/bench-host.sh" "${1:?mars|mercury}" "pvec64-$$"
OUT=${2:?output directory}; shift 2
ENTRIES=("$@")
[ ${#ENTRIES[@]} -gt 0 ] || ENTRIES=(pathological-1e3 architectural-1e3 architectural-1e5 mep-1e5 industrial-1e5 huge-coordinate-1e5 spline-heavy-1e5 architectural-1e6 spline-heavy-1e6)
mkdir -p "$OUT/$PLAT"
trap host_cleanup EXIT
host_push "$ROOT"
echo "== $BH_HOST ($PLAT): $("${SSH[@]}" uptime)"
rust "uname -m; rustc --version
      cargo build --release -q -p pvec-bench --benches 2>&1 | tail -3
      cargo bench -q -p pvec-bench --bench progressive -- header > /work/out/progressive.rows.csv; cat /proc/loadavg"
for E in "${ENTRIES[@]}"; do
  echo "== $E"
  rust "O=/work/out
        P() { cargo bench -q -p pvec-bench --bench progressive -- \"\$@\"; }
        P prepare $E \$O
        P run $E \$O $PLAT 3 >> \$O/progressive.rows.csv; rm -f \$O/$E.tier0.pvec; cat /proc/loadavg" 2>&1 | tail -2
done
fetch out/progressive.rows.csv > "$OUT/$PLAT/progressive.rows.csv"
echo "rows: $(wc -l < "$OUT/$PLAT/progressive.rows.csv") in $OUT/$PLAT"
