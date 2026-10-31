#!/usr/bin/env bash
# Checks the verify half of tools/wasm-release.sh against a made-up release directory. It needs no
# docker host: the build half is exercised by running tools/wasm-release.sh itself.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/wasm-release-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT

# the raster group of issue 221: a module names these exports or is not a release
RASTER_EXPORTS="raster_stat raster_skipped raster_placement raster_plan raster_tile raster_buffer_len raster_buffer_ptr raster_buffer_elem"
RASTER_JSON=$(printf '%s\n' $RASTER_EXPORTS | jq -R . | jq -sc .)
FAKE="not really wasm wasm_abi_version $RASTER_EXPORTS"
printf '%s' "$FAKE" > "$T/pvec_wasm_bg.wasm"
sha=$(shasum -a 256 "$T/pvec_wasm_bg.wasm" | cut -d' ' -f1)
size=$(wc -c < "$T/pvec_wasm_bg.wasm" | tr -d ' ')
cat > "$T/pvec-wasm-manifest.json" <<JSON
{
  "schema": 1,
  "crate": "pvec-wasm",
  "commit": "0000000000000000000000000000000000000000",
  "dirty": false,
  "abiVersion": 2,
  "rasterExports": $RASTER_JSON,
  "wasmBindgen": "0.2.129",
  "rustc": "rustc 1.99.1",
  "image": "rust:1.99.1-bookworm",
  "wasm": { "file": "pvec_wasm_bg.wasm", "bytes": $size, "sha256": "$sha" }
}
JSON
(cd "$T" && shasum -a 256 pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)

fail() { echo "FAIL: $*" >&2; exit 1; }

"$HERE/wasm-release.sh" --verify "$T" >/dev/null || fail "a consistent directory did not verify"

printf 'x' >> "$T/pvec_wasm_bg.wasm"
if "$HERE/wasm-release.sh" --verify "$T" >/dev/null 2>&1; then fail "a tampered wasm verified"; fi
printf '%s' "$FAKE" > "$T/pvec_wasm_bg.wasm"

# a module built without the raster group is refused, even with a manifest and sums that agree with it
for gone in raster_stat raster_tile raster_buffer_elem; do
  printf '%s' "${FAKE/$gone/}" > "$T/pvec_wasm_bg.wasm"
  g=$(shasum -a 256 "$T/pvec_wasm_bg.wasm" | cut -d' ' -f1); n=$(wc -c < "$T/pvec_wasm_bg.wasm" | tr -d ' ')
  jq --arg g "$g" --argjson n "$n" '.wasm.sha256 = $g | .wasm.bytes = $n' "$T/pvec-wasm-manifest.json" > "$T/m.json"
  cp "$T/pvec-wasm-manifest.json" "$T/manifest.keep"; cp "$T/m.json" "$T/pvec-wasm-manifest.json"
  (cd "$T" && shasum -a 256 pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)
  if "$HERE/wasm-release.sh" --verify "$T" >/dev/null 2>&1; then fail "a module without $gone verified"; fi
  cp "$T/manifest.keep" "$T/pvec-wasm-manifest.json"
done
printf '%s' "$FAKE" > "$T/pvec_wasm_bg.wasm"; rm -f "$T/m.json" "$T/manifest.keep"
(cd "$T" && shasum -a 256 pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)
"$HERE/wasm-release.sh" --verify "$T" >/dev/null || fail "the directory did not verify again after the raster cases"
jq 'del(.rasterExports)' "$T/pvec-wasm-manifest.json" > "$T/m.json"; cp "$T/pvec-wasm-manifest.json" "$T/manifest.keep"; cp "$T/m.json" "$T/pvec-wasm-manifest.json"
(cd "$T" && shasum -a 256 pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)
if "$HERE/wasm-release.sh" --verify "$T" >/dev/null 2>&1; then fail "a manifest that lists no raster exports verified"; fi
cp "$T/manifest.keep" "$T/pvec-wasm-manifest.json"; rm -f "$T/m.json" "$T/manifest.keep"
(cd "$T" && shasum -a 256 pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)

sed -i.bak 's/"dirty": false/"dirty": true/' "$T/pvec-wasm-manifest.json"; rm "$T/pvec-wasm-manifest.json.bak"
if "$HERE/wasm-release.sh" --verify "$T" >/dev/null 2>&1; then fail "an edited manifest verified"; fi
echo "wasm-release selftest (the verify half): ok"

# The probe half: tools/wasm-raster-probe.mjs calls the raster exports of a real staged module on the raster
# fixture, in a throw-away node container on the docker host (nothing runs here but ssh and tar). It needs a
# staged directory, so it runs only when one is given: tools/wasm-release-selftest.sh DIR
if [ -n "${1:-}" ]; then
  NAS_SH=${NAS_SH:-$HERE/../../tools/nas.sh}
  P="$T/probe"; mkdir -p "$P/tools"
  cp "$HERE/wasm-raster-probe.mjs" "$P/tools/"
  cp "$HERE/../tests/compat/raster-probe.pvec" "$P/raster-probe.pvec"
  probe() { # probe MODULE: 0 when the module passes
    cp "$1" "$P/module.wasm"
    name=wasm-probe-$$
    "$NAS_SH" push "$P" "$name" >/dev/null
    "$NAS_SH" run "$name" node:22-bookworm "node tools/wasm-raster-probe.mjs module.wasm raster-probe.pvec" 
    rc=$?; "$NAS_SH" clean "$name" >/dev/null 2>&1; return $rc
  }
  file=$(jq -er .wasm.file "$1/pvec-wasm-manifest.json")
  probe "$1/$file" || fail "the staged module fails the raster probe"
  # a module with no raster exports at all (an empty module that only exports the ABI version) fails it
  printf '\x00asm\x01\x00\x00\x00\x01\x05\x01\x60\x00\x01\x7f\x03\x02\x01\x00\x07\x14\x01\x10wasm_abi_version\x00\x00\x0a\x06\x01\x04\x00\x41\x02\x0b' > "$T/bare.wasm"
  if probe "$T/bare.wasm" >/dev/null 2>&1; then fail "a module built without the raster exports passed the probe"; fi
  echo "wasm-release selftest (the probe half): ok"
fi
