#!/usr/bin/env bash
# Runs the progressive coverage benchmark (pvec#64, P8.5) on one native host, in containers only. Per entry:
# `prepare` writes the corpus entry with a Tier 0 block per leaf, `run` measures the four stages on the three
# network arms plus the interaction arm and writes tidy rows to OUT/<platform>/progressive.rows.csv.
#
#   tools/progressive-bench.sh mars|mercury OUTDIR [ENTRY...]
#   an ENTRY such as architectural-1e6+r10 is the entry with ten scanned sheets over it (pvec#220)
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
# an entry named ENTRY+rN has N scanned sheets over it (pvec#220); the layout comparison runs once with them
if printf '%s\n' "${ENTRIES[@]}" | grep -q '+r'; then
  rust "cargo bench -q -p pvec-bench --bench progressive -- raster-layout $PLAT >> /work/out/progressive.rows.csv"
fi
fetch out/progressive.rows.csv > "$OUT/$PLAT/progressive.rows.csv"
echo "rows: $(wc -l < "$OUT/$PLAT/progressive.rows.csv") in $OUT/$PLAT"
