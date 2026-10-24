#!/usr/bin/env bash
# Runs the GML schema validation test (pvec#21) on MARS, where docker is.
#
#   tools/schema-validate.sh [ssh-target]      default devshell@192.168.0.100 (MARS, native x86_64)
#
# The test starts its own validator container, so the toolchain container it runs from needs the host's
# docker client and socket, and the work tree has to sit at the same path on the host and inside the
# container: the daemon resolves a bind-mount path on the host, and a /work that exists only inside the
# toolchain container gives the validator an empty mount (see the header of ../tools/nas.sh).
# Nothing but ssh, docker and writes to the one scratch directory happens on the host. The scratch
# tree and the cargo volumes are removed afterwards, from inside a container.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
NASSH=${NAS_SH:-$ROOT/../tools/nas.sh}
HOST=${1:-devshell@192.168.0.100}
IMAGE=${IMAGE:-pvec-rust:1.99.1}
NAME=${NAME:-pvec-schema-$$}
P=/work/nas-work/$NAME
SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST")

trap 'NAS=$HOST "$NASSH" clean "$NAME" >/dev/null 2>&1 || true; "${SSH[@]}" "docker volume rm pvec-$NAME-target pvec-$NAME-cargo" >/dev/null 2>&1 || true' EXIT
NAS=$HOST "$NASSH" push "$ROOT" "$NAME"
"${SSH[@]}" "docker run --rm --platform linux/amd64 \
  -v /var/run/docker.sock:/var/run/docker.sock -v /usr/bin/docker:/usr/bin/docker:ro \
  -v $P:$P -w $P -v pvec-$NAME-target:/tmp/target -v pvec-$NAME-cargo:/tmp/cargo \
  -e TMPDIR=$P/tmp -e CARGO_TARGET_DIR=/tmp/target -e CARGO_HOME=/tmp/cargo $IMAGE sh -c \
  'mkdir -p \$TMPDIR && uname -m && cargo test -p pvec-interchange --locked --test gml_21_schema -- --include-ignored --nocapture'"
