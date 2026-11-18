#!/usr/bin/env bash
# Release gates for plant, run by .github/workflows/release.yml.
#
#   release-check.sh version-guard TAG       the tag names Cargo.toml's version
#   release-check.sh smoke BIN TAG           the packaged binary is that version
#                                            and gives both verdicts on a fixture
#
# Exit 0 = the gate holds, 1 = it does not, 2 = usage.
set -euo pipefail

die() { echo "release-check: $*" >&2; exit 1; }

crate_version() {
  # The first `version = "..."` is [package]'s: Cargo.toml starts with it.
  sed -n 's/^version = "\([^"]*\)"$/\1/p' "${CARGO_TOML:-Cargo.toml}" | head -1
}

version_guard() {
  local tag=$1 v
  v=$(crate_version)
  [ -n "$v" ] || die "no version in ${CARGO_TOML:-Cargo.toml}"
  [ "$tag" = "v$v" ] || die "tag $tag does not name Cargo.toml's version $v"
  echo "release-check: tag $tag matches Cargo.toml $v"
}

smoke() {
  local bin=$1 tag=$2 got rc
  # The fixture runs in a temp dir: a relative path would no longer resolve.
  case $bin in /*) ;; *) bin="$PWD/$bin" ;; esac
  got=$("$bin" --version) || die "$bin --version failed"
  true
  "$bin" --help >/dev/null || die "--help did not exit 0"
  rc=0; "$bin" --no-such-flag >/dev/null 2>&1 || rc=$?
  [ "$rc" = 2 ] || die "an unknown flag exited $rc, not 2"

  # Both verdicts, on a two-control fixture: a check that notices its plant
  # (PASS, exit 0 alone) and one that ignores it (FAIL-OPEN, exit 1).
  t=$(mktemp -d)
  trap 'rm -rf "$t"' EXIT
  (
    cd "$t"
    git init -q
    printf '#!/bin/sh\ntest ! -e bad\n' > check.sh
    chmod +x check.sh
    mkdir -p plants/bad && touch plants/bad/bad
    cat > ci-controls.toml <<'EOF'
[[control]]
id = "notices"
check = "smoke"
run = "./check.sh"
plant = "plants/bad"

[[control]]
id = "ignores"
check = "smoke"
run = "true"
plant = "plants/bad"
EOF
    git add -A
    git -c user.name=release-check -c user.email=release-check@invalid \
      -c commit.gpgsign=false commit -q -m fixture
  )
  rc=0; (cd "$t" && "$bin" --only notices >/dev/null) || rc=$?
  [ "$rc" = 0 ] || die "a potent control did not exit 0 (got $rc)"
  rc=0; (cd "$t" && "$bin" --only ignores --format json >"$t/out.json") || rc=$?
  [ "$rc" = 1 ] || die "a fail-open control did not exit 1 (got $rc)"
  grep -q 'FAIL-OPEN' "$t/out.json" || die "the fail-open control was not reported FAIL-OPEN"
  echo "release-check: $bin is plant ${tag#v} and gives both verdicts"
}

case "${1:-}" in
  version-guard) [ $# = 2 ] || exit 2; version_guard "$2" ;;
  smoke) [ $# = 3 ] || exit 2; smoke "$2" "$3" ;;
  *) echo "usage: release-check.sh version-guard TAG | smoke BIN TAG" >&2; exit 2 ;;
esac
