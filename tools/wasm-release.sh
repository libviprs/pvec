#!/usr/bin/env bash
# Stages a loadable pvec-wasm module as a local release directory. Nothing is tagged, pushed or
# published: the directory is what a person attaches to a release by hand (see docs/WASM_ABI.md).
#
#   tools/wasm-release.sh [--out DIR] [--allow-dirty]   build and stage
#   tools/wasm-release.sh --verify DIR                  check a staged (or downloaded) directory
#
# The build runs in a throw-away container on the docker host through the shared wrapper, from
# whatever is checked out here, so it can be rerun on any commit. The wasm-bindgen version comes from
# Cargo.lock, never from this script. Environment: NAS_SH (wrapper), RUST_IMAGE (default
# rust:1.99.1-bookworm).
#
# The directory holds pvec_wasm_bg.wasm, pvec-wasm-manifest.json (commit, ABI version, sha256,
# toolchain) and SHA256SUMS. The default is dist/wasm-release/<commit>, which is git-ignored.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)

if [ "${1:-}" = "--verify" ]; then
  d=${2:?usage: wasm-release.sh --verify DIR}
  m="$d/pvec-wasm-manifest.json"
  (cd "$d" && shasum -a 256 -c SHA256SUMS >/dev/null) || { echo "SHA256SUMS does not match the files in $d" >&2; exit 1; }
  file=$(jq -er .wasm.file "$m"); want=$(jq -er .wasm.sha256 "$m"); bytes=$(jq -er .wasm.bytes "$m")
  have=$(shasum -a 256 "$d/$file" | cut -d' ' -f1)
  [ "$want" = "$have" ] || { echo "the manifest says $want, the module is $have" >&2; exit 1; }
  [ "$bytes" = "$(wc -c < "$d/$file" | tr -d ' ')" ] || { echo "the manifest byte count is wrong" >&2; exit 1; }
  jq -e '.abiVersion | type == "number"' "$m" >/dev/null || { echo "the manifest has no abiVersion" >&2; exit 1; }
  grep -q "pvec-wasm-manifest.json" "$d/SHA256SUMS" || { echo "SHA256SUMS does not cover the manifest" >&2; exit 1; }
  echo "ok $file $have abi $(jq -r .abiVersion "$m") commit $(jq -r .commit "$m")"
  exit 0
fi

OUT=""; DIRTY=0
while [ $# -gt 0 ]; do
  case $1 in
    --out) OUT=$2; shift 2 ;;
    --allow-dirty) DIRTY=1; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
NAS_SH=${NAS_SH:-$ROOT/../tools/nas.sh}
IMAGE=${RUST_IMAGE:-rust:1.99.1-bookworm}
COMMIT=$(git -C "$ROOT" rev-parse HEAD)
if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
  [ "$DIRTY" = 1 ] || { echo "the tree has uncommitted changes, commit them or pass --allow-dirty" >&2; exit 1; }
  IS_DIRTY=true
else
  IS_DIRTY=false
fi
OUT=${OUT:-$ROOT/dist/wasm-release/${COMMIT:0:12}}
NAME=pvec-wasm-release-$$
trap '"$NAS_SH" clean "$NAME" >/dev/null 2>&1 || true' EXIT

"$NAS_SH" push "$ROOT" "$NAME" >/dev/null
"$NAS_SH" run "$NAME" "$IMAGE" "
set -e
export CARGO_TARGET_DIR=/tmp/target CARGO_HOME=/tmp/cargo
rustup target add wasm32-unknown-unknown >/dev/null 2>&1
wb=\$(awk '/^name = \"wasm-bindgen\"\$/{getline; gsub(/[^0-9.]/,\"\"); print; exit}' Cargo.lock)
abi=\$(sed -n 's/^pub const WASM_ABI_VERSION: u32 = \([0-9]*\);.*/\1/p' crates/pvec-wasm/src/abi.rs)
[ -n \"\$wb\" ] && [ -n \"\$abi\" ] || { echo 'no wasm-bindgen version in Cargo.lock or no ABI constant' >&2; exit 1; }
cargo build --release --locked -p pvec-wasm --target wasm32-unknown-unknown 2>&1 | tail -1
cargo install wasm-bindgen-cli --version \"\$wb\" --locked >/dev/null 2>&1
/tmp/cargo/bin/wasm-bindgen --target web --no-typescript --out-dir /tmp/bg /tmp/target/wasm32-unknown-unknown/release/pvec_wasm.wasm
w=/tmp/bg/pvec_wasm_bg.wasm
grep -q wasm_abi_version \$w || { echo 'the processed module does not export wasm_abi_version' >&2; exit 1; }
mkdir -p /work/out && cp \$w /work/out/pvec_wasm_bg.wasm
sha=\$(sha256sum \$w | cut -d' ' -f1); bytes=\$(wc -c < \$w | tr -d ' ')
cat > /work/out/pvec-wasm-manifest.json <<JSON
{
  \"schema\": 1,
  \"crate\": \"pvec-wasm\",
  \"commit\": \"$COMMIT\",
  \"dirty\": $IS_DIRTY,
  \"abiVersion\": \$abi,
  \"wasmBindgen\": \"\$wb\",
  \"rustc\": \"\$(rustc --version)\",
  \"image\": \"$IMAGE\",
  \"wasm\": { \"file\": \"pvec_wasm_bg.wasm\", \"bytes\": \$bytes, \"sha256\": \"\$sha\" }
}
JSON
(cd /work/out && sha256sum pvec_wasm_bg.wasm pvec-wasm-manifest.json > SHA256SUMS)
" >&2

mkdir -p "$OUT"
ssh -o BatchMode=yes -o LogLevel=ERROR "${NAS:-devshell@192.168.0.100}" "tar -C /work/nas-work/$NAME/out -cf - ." | tar -C "$OUT" -xf -
"$0" --verify "$OUT"
echo "staged in $OUT (nothing was tagged or published)"
