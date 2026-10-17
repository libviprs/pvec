#!/bin/sh
# Counts instructions in the hot summing loops of the native release binary.
# usage: count_simd.sh <binary>
objdump -d -C --no-show-raw-insn "$1" > /tmp/dis.txt
for pat in 'packed::sum_f64' 'packed::sum_u32' 'fbs_soa::sum_iter' 'fbs_soa::sum_raw' 'fbs_soa::sum_ids' 'rk::sf' 'rk::si' 'FbObj.*touch'; do
  echo "--- $pat"
  awk -v pat="$pat" '
    /^[0-9a-f]+ <.*>:$/ { on = ($0 ~ pat); next }
    on && /^ +[0-9a-f]+:/ {
      n++; op = $2
      if (op ~ /^v?padd[qd]$/) padd++
      if ($0 ~ /xmm|ymm/) simd++
      if (op ~ /^v?(movdq[au]|movup[sd]|movaps)$/) wide++
    }
    END { print "instructions=" n+0 " packed_int_adds=" padd+0 " xmm_ymm_instructions=" simd+0 " wide_moves=" wide+0 }' /tmp/dis.txt
done
