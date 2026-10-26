# Sourced by the phase 8 benchmark drivers (tools/layout-bench.sh, tools/geometry-bench.sh). Sets up one native
# host for a benchmark run and defines the helpers; nothing runs on a host but ssh, docker and tar.
#
#   . tools/bench-host.sh mars|mercury NAME      sets SSH D PLATFORM PLAT MOUNT, defines host_push rust fetch host_cleanup
#
#   mars     native x86_64, bind-mounted scratch under /work/nas-work, image pvec-rust:1.99.1 (it has libhdf5)
#   mercury  native arm64 (Colima), a named volume per job, rust:1.99.1-bookworm
#
# `rust 'sh command'` runs the command in the Rust image with the tree at /work and the cargo caches inside it;
# `fetch FILE` prints a file from the tree; `host_cleanup` removes the scratch from inside a container (call it
# from a trap).
BH_HOST=${1:?mars|mercury}; BH_NAME=${2:?job name}
case $BH_HOST in
  mars)    SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR devshell@192.168.0.100); D=docker
           PLATFORM=linux/amd64; PLAT=linux-x86_64; MOUNT="/work/nas-work/$BH_NAME:"
           RUST_IMAGE=${RUST_IMAGE:-pvec-rust:1.99.1} ;;
  mercury) SSH=(ssh -o BatchMode=yes -o LogLevel=ERROR mercury); D=/opt/homebrew/bin/docker
           PLATFORM=linux/arm64; PLAT=linux-aarch64; MOUNT="$BH_NAME:"
           RUST_IMAGE=${RUST_IMAGE:-rust:1.99.1-bookworm} ;;
  *) echo "unknown host $BH_HOST" >&2; exit 2 ;;
esac

host_push() { # host_push ROOT
  if [ "$BH_HOST" = mars ]; then
    "${SSH[@]}" "mkdir -p /work/nas-work/$BH_NAME && chmod g+w /work/nas-work/$BH_NAME"
    (cd "$1" && git ls-files -co --exclude-standard -z | xargs -0 tar cf -) | "${SSH[@]}" "tar -xf - -m -C /work/nas-work/$BH_NAME"
    "${SSH[@]}" "mkdir -p /work/nas-work/$BH_NAME/out && chmod -R g+w /work/nas-work/$BH_NAME"
  else
    "${SSH[@]}" "$D volume create $BH_NAME" >/dev/null
    (cd "$1" && git ls-files -co --exclude-standard -z | xargs -0 tar cf -) | "${SSH[@]}" "$D run --rm -i --platform $PLATFORM -v $BH_NAME:/work alpine:3.20 sh -c 'tar xf - -C /work && mkdir -p /work/out'"
  fi
}

rust() {
  "${SSH[@]}" "$D run --rm --platform $PLATFORM -v ${MOUNT}/work -w /work -e CARGO_TARGET_DIR=/work/.target -e CARGO_HOME=/work/.cargo $RUST_IMAGE sh -c $(printf '%q' "$1")"
}

fetch() { # fetch PATH-IN-TREE
  "${SSH[@]}" "$D run --rm --platform $PLATFORM -v ${MOUNT}/vol alpine:3.20 sh -c 'cat /vol/$1'"
}

host_cleanup() {
  case $BH_HOST in
    mars) "${SSH[@]}" "mkdir -p /work/nas-work && $D run --rm --platform $PLATFORM -v /work/nas-work:/ws alpine:3.20 sh -c 'rm -rf /ws/$BH_NAME'" >/dev/null 2>&1 || true ;;
    mercury) "${SSH[@]}" "$D volume rm -f $BH_NAME" >/dev/null 2>&1 || true ;;
  esac
}
