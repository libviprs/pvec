#!/bin/sh
# The P7.4 measurement, run inside a rust:1.99.0-bookworm container (see docs/adr/ADR-P7.4-c-ffi-strategy.md).
# Prints one block per candidate. Nothing here writes outside /tmp.
set -eu
cd "$(dirname "$0")"
HERE=$(pwd)
export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo PVEC_B_HEADER_DIR=/tmp/hdr
T=/tmp/target/release
echo "== platform"
uname -m
rustc --version
gcc --version | head -1

echo "== fixtures"
cargo run -q --release --bin p74-fixture-gen -- /tmp/fix
cat /tmp/fix/expected.txt | wc -l
echo "== build"
s=$(date +%s)
cargo build -q --release --workspace
echo "workspace build seconds $(( $(date +%s) - s ))"

echo "== exported symbols (dynamic, defined, text)"
echo "a,b libpvec_ffi.so total $(nm -D --defined-only $T/libpvec_ffi.so | awk '$2=="T"' | wc -l) named pvec_ $(nm -D --defined-only $T/libpvec_ffi.so | awk '$2=="T"' | grep -c ' pvec_')"
echo "d   libpvec_ffi_d.so total $(nm -D --defined-only $T/libpvec_ffi_d.so | awk '$2=="T"' | wc -l) named pvec_d_ $(nm -D --defined-only $T/libpvec_ffi_d.so | awk '$2=="T"' | grep -c ' pvec_d_')"
echo "c   0 (no C ABI)"

echo "== header lines"
echo "a hand-written $(wc -l < ffi-ab/include/pvec_a.h) lines, $(grep -c '^[A-Za-z].*(.*);$' ffi-ab/include/pvec_a.h) declarations to keep in step"
echo "b generated $(wc -l < /tmp/hdr/pvec_b.h) lines, 0 written by hand"
echo "d hand-written $(wc -l < c/pvec_d.h) lines, $(grep -c '^[A-Za-z].*(.*);$' c/pvec_d.h) declarations to keep in step"

echo "== prototype outcome: every fixture opened from C, count compared with the generator's"
fail=0
while read -r name want; do
  f=/tmp/fix/$name.pvec
  c=$($T/p74-consumer-c "$f")
  gcc -Wall -Werror -DHEADER='"pvec_a.h"' -Iffi-ab/include c/count.c -L$T -lpvec_ffi -Wl,-rpath,$T -o /tmp/count_a
  a=$(/tmp/count_a "$f")
  gcc -Wall -Werror -DHEADER='"pvec_b.h"' -I/tmp/hdr c/count.c -L$T -lpvec_ffi -Wl,-rpath,$T -o /tmp/count_b
  b=$(/tmp/count_b "$f")
  gcc -Wall -Werror -Ic c/count_d.c -L$T -lpvec_ffi_d -Wl,-rpath,$T -o /tmp/count_d
  d=$(/tmp/count_d "$f")
  gcc -Wall -Werror -DHEADER='"pvec_a.h"' -Iffi-ab/include c/count.c $T/libpvec_ffi.a -lpthread -ldl -lm -o /tmp/count_a_static
  as=$(/tmp/count_a_static "$f")
  ok=PASS
  for got in "$c" "$a" "$b" "$d" "$as"; do [ "$got" = "$want" ] || ok=FAIL; done
  [ "$ok" = PASS ] || fail=1
  echo "$name want $want c=$c a=$a a_static=$as b=$b d=$d $ok"
done < /tmp/fix/expected.txt

echo "== experiment: drift. One parameter of pvec_entity_count narrows from u64* to u32* in Rust."
cp ffi-ab/src/lib.rs /tmp/lib.rs.orig
sed -i 's/out: \*mut u64) -> PvecStatus/out: *mut u32) -> PvecStatus/; s/unsafe { \*out = n };/unsafe { *out = n as u32 };/' ffi-ab/src/lib.rs
grep -c 'out: \*mut u32' ffi-ab/src/lib.rs
export CARGO_TARGET_DIR=/tmp/target-drift PVEC_B_HEADER_DIR=/tmp/hdr-drift
cargo build -q --release -p p74-ffi-ab
D=/tmp/target-drift/release
first=$(head -1 /tmp/fix/expected.txt | cut -d' ' -f1)
want=$(head -1 /tmp/fix/expected.txt | cut -d' ' -f2)
if gcc -Wall -Werror -DHEADER='"pvec_a.h"' -Iffi-ab/include c/count.c -L$D -lpvec_ffi -Wl,-rpath,$D -o /tmp/drift_a 2>/tmp/drift_a.err; then
  echo "a: the C program still compiles against the unchanged hand header, and prints $(/tmp/drift_a /tmp/fix/$first.pvec) where $want is right"
else
  echo "a: the C program no longer compiles"
fi
if gcc -Wall -Werror -DHEADER='"pvec_b.h"' -I/tmp/hdr-drift c/count.c -L$D -lpvec_ffi -Wl,-rpath,$D -o /tmp/drift_b 2>/tmp/drift_b.err; then
  echo "b: the C program still compiles"
else
  echo "b: the C program no longer compiles against the regenerated header: $(grep -m1 -o 'incompatible pointer type.*' /tmp/drift_b.err || head -1 /tmp/drift_b.err)"
fi
cp /tmp/lib.rs.orig ffi-ab/src/lib.rs

echo "== experiment: leak. A function that returns a Vec<u8> is added to the extern C layer."
export CARGO_TARGET_DIR=/tmp/target-leak PVEC_B_HEADER_DIR=/tmp/hdr-leak
cp ffi-d/src/lib.rs /tmp/libd.rs.orig
printf '\n#[unsafe(no_mangle)]\npub extern "C" fn pvec_leak() -> Vec<u8> {\n    Vec::new()\n}\n' >> ffi-ab/src/lib.rs
printf '\n#[unsafe(no_mangle)]\npub extern "C" fn pvec_d_leak() -> Vec<u8> {\n    Vec::new()\n}\n' >> ffi-d/src/lib.rs
for pkg in p74-ffi-ab p74-ffi-d; do
  if cargo build -q --release -p $pkg 2>/tmp/leak.err; then echo "$pkg: builds with the deny lint on"; else echo "$pkg: rustc refuses: $(grep -m1 -o 'error: .*' /tmp/leak.err)"; fi
done
echo "b: cbindgen alone put it in the header: $(grep -c 'Vec_u8' /tmp/hdr-leak/pvec_b.h) mentions of Vec_u8"
sed -i 's/^#!\[deny(improper_ctypes_definitions)\]$//' ffi-ab/src/lib.rs
if cargo build -q --release -p p74-ffi-ab 2>/tmp/leak2.err; then echo "a,b: with the deny lint removed it builds, with $(grep -c 'improper_ctypes_definitions' /tmp/leak2.err) warning lines"; else echo "a,b: still refused"; fi
cp /tmp/lib.rs.orig ffi-ab/src/lib.rs
cp /tmp/libd.rs.orig ffi-d/src/lib.rs

echo "== candidate (c): the WASM interface that stands in for a C ABI"
export CARGO_TARGET_DIR=/tmp/target-wasm
rustup target add wasm32-unknown-unknown >/dev/null 2>&1
(cd ../.. && cargo build -q --release -p pvec-wasm --target wasm32-unknown-unknown)
W=/tmp/target-wasm/wasm32-unknown-unknown/release/pvec_wasm.wasm
echo "pvec_wasm.wasm $(wc -c < $W) bytes"
export CARGO_TARGET_DIR=/tmp/target
$T/p74-wasm-exports "$W" | sed 's/^/exports /'
[ "$fail" = 0 ] && echo "== all prototypes agree" || { echo "== PROTOTYPES DISAGREE"; exit 1; }
