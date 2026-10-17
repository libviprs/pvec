#!/usr/bin/env bash
# Builds the gate image on MARS: the official Rust image plus clippy, rustfmt and the wasm32
# target, so a gate run does not re-download components every time.
#   tools/build-image.sh [rust-version]      (default 1.99.1)
#   tools/build-image.sh --untag [version]   remove it again when the work is done
set -euo pipefail
NAS=${NAS:-devshell@192.168.0.100}
if [ "${1:-}" = "--untag" ]; then
  V=${2:-1.99.1}
  ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS" "docker rmi pvec-rust:$V"
  exit 0
fi
V=${1:-1.99.1}
ssh -o BatchMode=yes -o LogLevel=ERROR "$NAS" "docker build -q --platform linux/amd64 -t pvec-rust:$V - <<EOF2
FROM rust:$V-bookworm
RUN rustup component add clippy rustfmt && rustup target add wasm32-unknown-unknown
EOF2"
