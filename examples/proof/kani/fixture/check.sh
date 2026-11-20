#!/bin/sh
# The fixture's proof gate: run the Kani harnesses, then check that every
# kani::assume in the code is in assumes.ledger. Comment lines are not code
# and are ignored.
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
echo "ledger: OK ($(printf '%s\n' "$found" | grep -c .) assume(s), all listed)"
