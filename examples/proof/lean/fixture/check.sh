#!/bin/sh
# The fixture's proof gate: build the library, then audit the axioms of every
# checked theorem. Only Lean's three standard axioms are allowed; sorryAx (a
# `sorry`) or any other axiom fails the gate.
set -e
lake build
out=$(lake env lean Audit.lean)
printf '%s\n' "$out"
bad=$(printf '%s\n' "$out" | tr ',[]' '\n\n\n' | sed 's/.*depends on axioms://; s/^ *//; s/ *$//' \
  | grep -v -e '^$' -e "does not depend on any axioms" -e '^propext$' -e '^Classical.choice$' -e '^Quot.sound$' || true)
if [ -n "$bad" ]; then
  echo "axiom audit: disallowed axiom(s):" $bad
  exit 1
fi
echo "axiom audit: OK"
