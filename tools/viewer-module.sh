#!/bin/sh
# Runs inside the Rust container that tools/viewer-test.sh starts: builds the loadable pvec-wasm module the way
# tools/wasm-release.sh does (wasm-bindgen at the Cargo.lock version, --target web) and puts it in viewer/.
set -e
export CARGO_HOME=${CARGO_HOME:-/opt/cargo} CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/opt/target}
rustup target add wasm32-unknown-unknown >/dev/null 2>&1
wb=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/[^0-9.]/,""); print; exit}' Cargo.lock)
[ -n "$wb" ] || { echo "no wasm-bindgen version in Cargo.lock" >&2; exit 1; }
cargo build --release --locked -p pvec-wasm --target wasm32-unknown-unknown 2>&1 | tail -1
if [ "$("$CARGO_HOME/bin/wasm-bindgen" --version 2>/dev/null)" != "wasm-bindgen $wb" ]; then
  cargo install wasm-bindgen-cli --version "$wb" --locked --force >/dev/null 2>&1
fi
"$CARGO_HOME/bin/wasm-bindgen" --target web --no-typescript --out-dir /tmp/bg \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/pvec_wasm.wasm"
cp /tmp/bg/pvec_wasm_bg.wasm viewer/pvec_wasm_bg.wasm
echo "module: $(wc -c < viewer/pvec_wasm_bg.wasm) bytes, wasm-bindgen $wb, $(rustc --version), $(uname -m)"
