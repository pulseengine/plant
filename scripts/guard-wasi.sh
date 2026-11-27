#!/bin/sh
# plant guard as a WASI component (REQ-018): run the wasm32-wasip2 build under
# wasmtime and require the same stdout and exit code as the native binary on
# plant's own workflows and the guard-wasi fixtures. Run from the repo root:
#   scripts/guard-wasi.sh target/wasm32-wasip2/release/plant.wasm target/debug/plant
set -eu
wasm=${1:?usage: guard-wasi.sh PLANT_WASM PLANT_NATIVE}
native=${2:?usage: guard-wasi.sh PLANT_WASM PLANT_NATIVE}
runtime=${WASM_RUNTIME:-wasmtime run -S cli-exit-with-code=y --dir .::.}
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
fail=0
n=0

# case WANT ARGS...: both builds must exit WANT and print the same stdout.
case_() {
  want=$1
  shift
  n=$((n + 1))
  set +e
  $runtime "$wasm" "$@" >"$out/w" 2>"$out/we"
  w=$?
  "$native" "$@" >"$out/n" 2>"$out/ne"
  c=$?
  set -e
  if [ "$w" != "$want" ] || [ "$c" != "$want" ]; then
    echo "FAIL  plant $*: wasm exit $w, native exit $c, want $want"
    sed 's/^/      wasm stderr: /' "$out/we"
    fail=1
  elif ! cmp -s "$out/w" "$out/n"; then
    echo "FAIL  plant $*: stdout differs"
    diff "$out/n" "$out/w" | sed 's/^/      /'
    fail=1
  else
    echo "ok    plant $* (exit $want)"
  fi
}

case_ 0 --version
case_ 0 guard .
case_ 0 guard --format json .
case_ 1 guard tests/fixtures/guard-wasi/unguarded
case_ 1 guard --format json tests/fixtures/guard-wasi/unguarded
case_ 2 guard tests/fixtures/guard-wasi/bad-config
case_ 2 guard --bogus

if [ "$fail" != 0 ]; then
  echo "guard-wasi: the WASI build disagrees with the native binary"
  exit 1
fi
echo "guard-wasi: $n of $n cases agree"
