#!/usr/bin/env bash
# Pin check (pvec#304): every consumer of pvec has to build against one pvec rev.
#
#   tools/pin-check.sh <checkout> [<checkout> ...]
#
# For each checkout it reads the pvec rev of every dependency on https://github.com/libviprs/pvec in
#   Cargo.toml, tests/pdf-compare/gen/Cargo.toml (when present)
# and compares them with pvec.pin.json (when present). It prints one line per file and rev, then a verdict.
# Exit 0: all revs are the same. Exit 1: a mismatch, a pvec dependency without a rev, or a pvec.pin.json that
# disagrees with Cargo.toml. Exit 2: bad arguments or no pvec dependency found anywhere. Reads only, writes nothing.
# A checkout that has no pvec dependency yet is reported and skipped, as long as another one has.
set -euo pipefail

[ $# -ge 1 ] || { echo "usage: $0 <checkout> [<checkout> ...]" >&2; exit 2; }

fail=0
revs=()
found=0

# One line per pvec dependency: crate<TAB>rev. The crate is the `package = ` name when there is one.
deps() {
  awk '
    /^[[:space:]]*#/ { next }
    /github\.com\/libviprs\/pvec/ {
      key = $1; sub(/[[:space:]]*=.*/, "", key)
      line = $0
      pkg = key
      if (match(line, /package[[:space:]]*=[[:space:]]*"[^"]*"/)) { pkg = substr(line, RSTART, RLENGTH); sub(/^[^"]*"/, "", pkg); sub(/"$/, "", pkg) }
      rev = "-"
      if (match(line, /rev[[:space:]]*=[[:space:]]*"[^"]*"/)) { rev = substr(line, RSTART, RLENGTH); sub(/^[^"]*"/, "", rev); sub(/"$/, "", rev) }
      print pkg "\t" rev
    }' "$1"
}

for dir in "$@"; do
  [ -d "$dir" ] || { echo "not a directory: $dir" >&2; exit 2; }
  name=$(basename "$(cd "$dir" && pwd)")
  tomls=0
  for rel in Cargo.toml tests/pdf-compare/gen/Cargo.toml; do
    f="$dir/$rel"
    [ -f "$f" ] || continue
    list=$(deps "$f")
    [ -n "$list" ] || continue
    tomls=$((tomls + 1)); found=1
    while IFS=$'\t' read -r crate rev; do
      printf '%-14s %-34s %-20s %s\n' "$name" "$rel" "$crate" "$rev"
      if [ "$rev" = "-" ]; then
        echo "  MISMATCH: $crate in $name/$rel has no rev" >&2; fail=1
      else
        revs+=("$rev")
      fi
    done <<< "$list"
  done
  [ "$tomls" -gt 0 ] || echo "$name: no pvec dependency, skipped"

  pin="$dir/pvec.pin.json"
  if [ -f "$pin" ] && [ -f "$dir/Cargo.toml" ]; then
    commit=$(sed -n 's/.*"commit"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$pin" | head -1)
    jcrates=$(tr -d '\n' < "$pin" | sed -n 's/.*"crates"[[:space:]]*:[[:space:]]*\[\([^]]*\)\].*/\1/p' | tr ',' '\n' | tr -d ' "' | sort -u)
    ccrates=$(deps "$dir/Cargo.toml" | cut -f1 | sort -u)
    printf '%-14s %-34s %-20s %s\n' "$name" pvec.pin.json commit "$commit"
    [ -z "$commit" ] || revs+=("$commit")
    missing=$(comm -13 <(printf '%s\n' "$jcrates") <(printf '%s\n' "$ccrates") | tr '\n' ' ')
    extra=$(comm -23 <(printf '%s\n' "$jcrates") <(printf '%s\n' "$ccrates") | tr '\n' ' ')
    if [ -n "$missing" ] || [ -n "$extra" ]; then
      echo "  MISMATCH: $name/pvec.pin.json crates differ from Cargo.toml (not in the json: ${missing:-none}; only in the json: ${extra:-none})" >&2
      fail=1
    fi
  fi
done

[ "$found" -eq 1 ] || { echo "no pvec dependency found in any checkout" >&2; exit 2; }

distinct=$(printf '%s\n' ${revs[@]+"${revs[@]}"} | sort -u)
if [ "$(printf '%s\n' "$distinct" | wc -l | tr -d ' ')" -gt 1 ]; then
  echo "MISMATCH: $(printf '%s\n' "$distinct" | wc -l | tr -d ' ') different pvec revs:" >&2
  printf '%s\n' "$distinct" | while read -r r; do
    echo "  $r" >&2
  done
  fail=1
fi

if [ "$fail" -eq 0 ]; then echo "OK: one pvec rev ($distinct)"; else echo "FAIL" >&2; fi
exit "$fail"
