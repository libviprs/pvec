#!/usr/bin/env bash
# The P8.6 regression harness (pvec#65): what runs on every tuning commit. It re-runs the phase 2 conformance
# suites, the phase 4 limit tests and the P7.8 capability matrix and fuzz seed runs, then the headline number of
# each of P8.2 to P8.5, and writes one `key=value` file with all of it.
#
#   tools/tune-check.sh native|mars|mercury OUTDIR [LABEL]
#
# `native` runs on this machine with its own cargo, for the loop while a parameter is being moved (a Mac result,
# never a number for the log). `mars` (native x86_64) and `mercury` (native arm64) run in a container through the
# same helpers the benchmark drivers use, and those are the numbers that go in docs/bench/TUNING.md.
#
# The suites are the ones the issue names: P2.5 (#38) and P2.6 (#39) in pvec-harness, P2.7 (#40) in pvec-testkit
# and pvec-conformance, P4.8 (#9) in pvec-container, P7.8 (#59) in pvec-fuzz. The headline numbers are the viewport
# p95 and bytes of the .pvec arm of P8.2 (vs_step), the total viewport bytes and the SoA decode ratio of P8.3
# (layout), ns per segment of P8.4 (geometry) and the four stage times of P8.5 (progressive).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MODE=${1:?native|mars|mercury}; OUT=${2:?output directory}; LABEL=${3:-$(git -C "$ROOT" rev-parse --short HEAD)}
SUITES=$(printf '%s ' P2.5:pvec-harness:issue_38_structural P2.6:pvec-harness:issue_39_geometric P2.8:pvec-harness:issue_41_gates \
  P2.7:pvec-testkit:issue_40_hostile P2.7:pvec-conformance:hostile P4.8:pvec-container:issue_9_limits \
  P7.8m:pvec-fuzz:capability_matrix P7.8f:pvec-fuzz:targets P7.8f:pvec-fuzz:smoke P7.8f:pvec-fuzz:seeds \
  P7.8f:pvec-fuzz:decompress P7.8f:pvec-fuzz:mutator P7.8f:pvec-fuzz:gate P7.8f:pvec-fuzz:regressions)

# the commands run inside the tree: $1 is the scratch directory, $2 the platform label, $3 the host label
INNER='
set -e
S=$1; PLAT=$2; HOST=$3; mkdir -p $S
echo "label='"$LABEL"'"; echo "platform=$PLAT"; echo "host=$HOST"
for s in '"$SUITES"'; do
  IFS=: read -r id crate test <<< "$s"
  r=$(cargo test --release --locked -p $crate --test $test 2>&1 | grep -E "^test result" || true)
  ok=$(echo "$r" | awk "/ok\./ {n+=\$4} END {print n+0}"); bad=$(echo "$r" | awk "/FAILED/ {n+=\$6} END {print n+0}")
  if [ -n "$r" ] && [ "$bad" = 0 ]; then echo "suite.$id.$crate.$test=pass $ok"; else echo "suite.$id.$crate.$test=FAIL ok=$ok failed=$bad"; fi
done
cargo build --release -q -p pvec-bench --benches 2>&1 | tail -3
P() { cargo bench -q -p pvec-bench --bench progressive -- "$@" 2>/dev/null; }
L() { cargo bench -q -p pvec-bench --bench layout -- "$@" 2>/dev/null; }
V() { cargo bench -q -p pvec-bench --bench vs_step -- "$@" 2>/dev/null; }
G() { cargo bench -q -p pvec-bench --bench geometry -- "$@" 2>/dev/null; }
for E in architectural-1e6 huge-coordinate-1e5; do
  P header > $S/p.csv; P prepare $E $S; P run $E $S $PLAT 3 >> $S/p.csv; rm -f $S/$E.tier0.pvec
  for arm in 25mbit-60ms 5mbit-200ms; do
    for m in directory_s tier0_s refine_full_s; do echo "p85.$E.$arm.$m=$(grep "^stage,$E,$PLAT,$arm,p95,$m," $S/p.csv | cut -d, -f7)"; done
  done
  echo "p85.$E.coverage_min=$(grep "^stage,$E,$PLAT,25mbit-60ms,min,coverage," $S/p.csv | cut -d, -f7)"
  echo "p85.$E.preview_px_max=$(grep "^stage,$E,$PLAT,25mbit-60ms,max,worst_preview_px," $S/p.csv | cut -d, -f7)"
  echo "p85.$E.pan_discard=$(grep "^interaction,$E,$PLAT,25mbit-60ms,pan,discard_share," $S/p.csv | cut -d, -f7)"
  echo "p85.$E.zoom_discard=$(grep "^interaction,$E,$PLAT,25mbit-60ms,zoom,discard_share," $S/p.csv | cut -d, -f7)"
  cp $S/p.csv $S/progressive.$E.csv
done
for E in architectural-1e6 spline-heavy-1e5; do
  V prepare $E $S >/dev/null; V pvec $E $S >/dev/null
  grep -E "^(viewport_p95_ms|viewport_bytes_p95|viewport_bytes_max|viewport_ranges_max|cold_open_ms)=" $S/$E.pvec.kv | sed "s/^/p82.$E./"
  echo "p82.$E.file_bytes=$(stat -c %s $S/$E.pvec 2>/dev/null || stat -f %z $S/$E.pvec)"
  rm -f $S/$E.stp $S/$E.shapes $S/$E.pvec
done
E=architectural-1e6
L header > $S/l.csv; L prepare $E $S; { L decode $E $S $PLAT; L shipped $E $S $PLAT; } >> $S/l.csv
echo "p83.$E.bytes_total_uncompressed_layout=$(grep "^index,$E,$PLAT,shipped,all,bytes_total," $S/l.csv | cut -d, -f7)"
echo "p83.$E.ranges_total=$(grep "^index,$E,$PLAT,shipped,all,ranges_total," $S/l.csv | cut -d, -f7)"
soa=$(grep "^decode,$E,$PLAT,soa,,entities_per_s," $S/l.csv | cut -d, -f7); rec=$(grep "^decode,$E,$PLAT,record,,entities_per_s," $S/l.csv | cut -d, -f7)
echo "p83.$E.soa_entities_per_s=$soa"; echo "p83.$E.record_entities_per_s=$rec"; echo "p83.$E.soa_over_record=$(awk "BEGIN {print $soa/$rec}")"
rm -f $S/$E.pvec $S/$E.shapes
G header > $S/g.csv; G run $PLAT $HOST >> $S/g.csv
echo "p84.ns_per_segment_mean=$(awk -F, "\$7==\"ns_per_segment\" && \$8!=\"\" {s+=\$8; n++} END {if (n) print s/n}" $S/g.csv)"
echo "p84.rows=$(wc -l < $S/g.csv | tr -d ' ')"
'

mkdir -p "$OUT"
case $MODE in
  native)
    PLAT=native-$(uname -s)-$(uname -m)
    SCR="$OUT/work-$LABEL"; mkdir -p "$SCR"
    (cd "$ROOT" && LABEL=$LABEL bash -c "$INNER" _ "$SCR" "$PLAT" "$(hostname -s)") | tee "$OUT/tune-$LABEL-native.kv" ;;
  mars|mercury)
    . "$ROOT/tools/bench-host.sh" "$MODE" "pvec65-$$-$LABEL"
    trap host_cleanup EXIT
    host_push "$ROOT"
    rust "$(printf 'LABEL=%q; ' "$LABEL")export LABEL; bash -c $(printf '%q' "$INNER") _ /work/out/tune $PLAT $BH_HOST" | tee "$OUT/tune-$LABEL-$PLAT.kv" ;;
  *) echo "unknown mode $MODE" >&2; exit 2 ;;
esac
