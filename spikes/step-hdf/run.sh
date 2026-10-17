#!/bin/sh
# Runs every measurement for one fixture inside the container. Output files go under ./out of the
# scratch tree, which the wrapper removes afterwards.
#   sh run.sh <fixture> <all|subset>
set -e
F=$1
MODE=${2:-subset}
B=/tmp/target/release/measure
O=$(pwd)/out
mkdir -p "$O"
cargo build --release -q
echo "load average at start: $(cat /proc/loadavg)"
echo "## sizes: $F"
echo "| fixture | layout | config | file bytes | vs Part 21 | logical payload | file over payload | file minus dataset storage | (that) over payload | write s |"
echo "|---|---|---|---|---|---|---|---|---|---|"
for l in h1 h2; do
  if [ "$MODE" = all ]; then
    $B sizes "$F" $l all "$O"
  else
    for c in "contiguous" "contiguous+latest" "chunk 65536 + shuffle + deflate 6" "chunk 4096 + shuffle + deflate 6 + latest"; do
      $B sizes "$F" $l "$c" "$O"
    done
  fi
done
for c in contiguous chunk_65536_p_shuffle_p_deflate_6 chunk_4096_p_shuffle_p_deflate_6_p_latest; do
  for l in h1 h2; do
    P="$O/$F-$l-$c.h5"
    echo; echo "## $F $l $c"
    $B verify "$F" $l "$P"
    $B viewport "$F" $l "$P"
    $B decode $l "$P" 0
    $B extract "$F" $l "$P"
    $B progressive "$F" $l "$P"
  done
done
