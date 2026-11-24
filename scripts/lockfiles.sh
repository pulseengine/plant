#!/bin/sh
# Lockfile gate: every Cargo workspace in the tree has a committed Cargo.lock,
# cargo-audit reads each one, and each one is binding (`cargo metadata
# --locked` would not change it).
#
# Workspaces are found from the files, not from a list: every Cargo.toml
# outside target/ is resolved to its workspace root by cargo. A nested
# workspace (examples/sieve/fixture/tools) is therefore audited without being
# named here. Controls in ci-controls.lockfiles.toml plant a stale, a missing
# and a vulnerable nested lock that this script must reject.
#
# Usage: scripts/lockfiles.sh   (from the repository root)
# Exit: 0 all locks committed, audited clean and binding; 1 otherwise, or no
# workspace found (nothing compared).
set -u

roots=$(find . -name Cargo.toml -not -path '*/target/*' -not -path './.git/*' -not -path './ci-controls/*' -not -path './lock-controls/*' |
  while read -r m; do
    cargo locate-project --workspace --message-format plain --manifest-path "$m"
  done | sort -u)

if [ -z "$roots" ]; then
  echo "lockfiles: no Cargo workspace found; nothing compared"
  exit 1
fi

in_git=false
git rev-parse --is-inside-work-tree >/dev/null 2>&1 && in_git=true

fail=0
n=0
for root in $roots; do
  n=$((n + 1))
  dir=$(dirname "$root")
  lock="$dir/Cargo.lock"
  rel=${lock#"$PWD"/}
  if [ ! -f "$lock" ]; then
    echo "lockfiles: $rel: no Cargo.lock for workspace $root"
    fail=1
    continue
  fi
  if $in_git && ! git ls-files --error-unmatch "$lock" >/dev/null 2>&1; then
    echo "lockfiles: $rel: not committed (untracked or gitignored)"
    fail=1
  fi
  # A fetch failure of the advisory database is not a finding: say which it
  # was. Both fail the gate; an unaudited lock never passes.
  out=$(cargo audit --quiet --deny warnings --file "$lock" 2>&1)
  rc=$?
  [ -n "$out" ] && printf '%s\n' "$out"
  if [ "$rc" -ne 0 ]; then
    case $out in
      *"couldn't fetch advisory database"*)
        echo "lockfiles: $rel: advisory database unreachable; the lock was not audited" ;;
      *)
        echo "lockfiles: $rel: cargo audit reported findings" ;;
    esac
    fail=1
  fi
  if ! cargo metadata --locked --format-version 1 --manifest-path "$root" >/dev/null; then
    echo "lockfiles: $rel: not binding: cargo would change it"
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "lockfiles: OK, $n workspace(s): committed, audited, binding"
