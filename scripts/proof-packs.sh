#!/usr/bin/env bash
# Runs the proof-gate packs in examples/proof and asserts each manifest's
# expected verdicts:
#   ci-controls.toml           every control PASS (exit 0)
#   ci-controls.stand-in.toml  every control FAIL-OPEN (a gate that only
#                              type-checks must not pass the pack)
#   ci-controls.blind.toml     every control FAIL-OPEN (a hole in a file the
#                              gate never builds must not count as rejected)
#
#   scripts/proof-packs.sh PLANT_BIN [PACK...]     default packs: lean
#
# Each fixture is copied into a throwaway git repository, since plant runs
# on HEAD. Exit 0 when every manifest gave its expected verdicts, 1 otherwise,
# 2 on usage.
set -euo pipefail

plant=${1:?usage: proof-packs.sh PLANT_BIN [PACK...]}; shift
case $plant in /*) ;; *) plant="$PWD/$plant" ;; esac
root=$(cd "$(dirname "$0")/.." && pwd)
packs=("$@"); [ ${#packs[@]} -gt 0 ] || packs=(lean)

t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail=0
for pack in "${packs[@]}"; do
  src="$root/examples/proof/$pack/fixture"
  [ -d "$src" ] || { echo "proof-packs: no pack $pack" >&2; exit 2; }
  work="$t/$pack"
  cp -R "$src" "$work"
  rm -rf "$work/.lake"
  (
    cd "$work"
    git init -q
    git add -A
    git -c user.name=proof-packs -c user.email=proof-packs@invalid \
      -c commit.gpgsign=false commit -q -m fixture
  )
  for m in ci-controls.toml ci-controls.stand-in.toml ci-controls.blind.toml; do
    [ -f "$work/$m" ] || continue
    want_rc=1; want_pass=0
    [ "$m" = ci-controls.toml ] && { want_rc=0; want_pass=1; }
    n=$(grep -c '^\[\[control\]\]' "$work/$m")
    rc=0; out=$(cd "$work" && "$plant" "$m") || rc=$?
    printf '%s\n' "$out" | sed "s/^/[$pack $m] /"
    if [ "$want_pass" = 1 ]; then
      got=$(printf '%s\n' "$out" | grep -c '^PASS ' || true)
    else
      got=$(printf '%s\n' "$out" | grep -c 'FAIL-OPEN' || true)
    fi
    if [ "$rc" != "$want_rc" ] || [ "$got" != "$n" ]; then
      echo "proof-packs: $pack/$m: exit $rc (want $want_rc), $got of $n controls as expected" >&2
      fail=1
    else
      echo "proof-packs: $pack/$m: as expected ($n of $n)"
    fi
  done
done
exit "$fail"
