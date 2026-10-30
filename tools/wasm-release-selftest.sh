#!/usr/bin/env bash
# Checks the verify half of tools/wasm-release.sh against a made-up release directory. It needs no
# docker host: the build half is exercised by running tools/wasm-release.sh itself.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/wasm-release-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT

printf 'not really wasm' > "$T/pvec_wasm_bg.wasm"
sha=$(shasum -a 256 "$T/pvec_wasm_bg.wasm" | cut -d' ' -f1)
cat > "$T/pvec-wasm-manifest.json" <<JSON
{
  "schema": 1,
  "crate": "pvec-wasm",
  "commit": "0000000000000000000000000000000000000000",
  "dirty": false,
  "abiVersion": 1,
  "wasmBindgen": "0.2.129",
  "rustc": "rustc 1.99.1",
  "image": "rust:1.99.1-bookworm",
  "wasm": { "file": "pvec_wasm_bg.wasm", "bytes": 15, "sha256": "$sha" }
}
JSON
(cd "$T" && shasum -a 256 pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)

fail() { echo "FAIL: $*" >&2; exit 1; }

"$HERE/wasm-release.sh" --verify "$T" >/dev/null || fail "a consistent directory did not verify"

printf 'x' >> "$T/pvec_wasm_bg.wasm"
if "$HERE/wasm-release.sh" --verify "$T" >/dev/null 2>&1; then fail "a tampered wasm verified"; fi
printf 'not really wasm' > "$T/pvec_wasm_bg.wasm"

sed -i.bak 's/"dirty": false/"dirty": true/' "$T/pvec-wasm-manifest.json"; rm "$T/pvec-wasm-manifest.json.bak"
if "$HERE/wasm-release.sh" --verify "$T" >/dev/null 2>&1; then fail "an edited manifest verified"; fi
echo "wasm-release selftest: ok"
