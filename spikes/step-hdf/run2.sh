#!/bin/sh
# Second pass: decode and extraction after the dense-reader fix, and viewport reads with a larger
# HDF5 chunk cache for the compressed configurations.
#   sh run2.sh <fixture>
set -e
F=$1
B=/tmp/target/release/measure
O=$(pwd)/out
mkdir -p "$O"
cargo build --release -q
echo "load average at start: $(cat /proc/loadavg)"
for c in "contiguous" "chunk 65536 + shuffle + deflate 6" "chunk 4096 + shuffle + deflate 6 + latest"; do
  for l in h1 h2; do $B sizes "$F" $l "$c" "$O" > /dev/null; done
done
for c in contiguous chunk_65536_p_shuffle_p_deflate_6 chunk_4096_p_shuffle_p_deflate_6_p_latest; do
  for l in h1 h2; do
    P="$O/$F-$l-$c.h5"
    echo; echo "## $F $l $c"
    $B decode $l "$P" 0
    $B extract "$F" $l "$P"
    case $c in
      chunk_65536*) echo "viewport with a 256 MiB chunk cache"; H5_CHUNK_CACHE_MB=256 $B viewport "$F" $l "$P" ;;
    esac
  done
done
