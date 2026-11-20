#!/bin/sh
# The fixture's proof gate: run the Kani harnesses, then check that every
# kani::assume in the code is in assumes.ledger. Comment lines are not code
# and are ignored. Last, the ledger may only shrink: it is compared with
# HEAD's copy, so the control needs a base commit (shallow_git = true).
set -e
cargo kani --output-format terse
found=$(grep -rn 'kani::assume' src | grep -v '^[^:]*:[0-9]*:[[:space:]]*//' \
  | sed 's/^\([^:]*\):[0-9]*:[[:space:]]*/\1: /' | sort -u)
listed=$(grep -v '^#' assumes.ledger | grep . | sort -u)
extra=$(printf '%s\n' "$found" | while IFS= read -r a; do printf '%s\n' "$listed" | grep -qxF "$a" || printf '%s\n' "$a"; done)
if [ -n "$extra" ]; then
  echo "ledger: assume not in assumes.ledger:"
  printf '  %s\n' "$extra"
  exit 1
fi
base=$(git show HEAD:assumes.ledger) || { echo "ledger: no base commit to compare with"; exit 1; }
was=$(printf '%s\n' "$base" | grep -v '^#' | grep -c . || true)
now=$(grep -v '^#' assumes.ledger | grep -c . || true)
if [ "$now" -gt "$was" ]; then
  echo "ledger: grew from $was to $now entries; it may only shrink"
  exit 1
fi
echo "ledger: OK ($(printf '%s\n' "$found" | grep -c .) assume(s), all listed)"
