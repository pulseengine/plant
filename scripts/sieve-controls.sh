#!/usr/bin/env bash
# Runs examples/sieve's false-skip controls against the pinned sieve release.
#
#   scripts/sieve-controls.sh PLANT_BIN
#
# The archive is fetched by URL and checked against a digest pinned here. The
# digests are copied from sieve v0.2.0's SHA256SUMS.txt, which was verified
# with cosign against
# https://github.com/pulseengine/sieve/.github/workflows/release.yml@refs/tags/v0.2.0
# when this pin was set. Moving the pin is a reviewed change to this file.
# The fixture is copied into a throwaway git repository, since plant runs on
# HEAD, and plant runs there with this sieve first on PATH.
#
# Exit: plant's own exit code; 2 if sieve cannot be fetched or does not match.
set -euo pipefail

SIEVE_VERSION=v0.2.0

plant=$1; shift
case $plant in /*) ;; *) plant="$PWD/$plant" ;; esac
root=$(cd "$(dirname "$0")/.." && pwd)

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl
    sha=c052a0b6342955dfb546baf32430b5a408fd42b472f33bc63fb4fc61dc6d637c ;;
  Linux-aarch64) target=aarch64-unknown-linux-musl
    sha=8a8ca4b9862e59b33c8fcf39944cd822af9b72de0ce1e956a252b293d02df083 ;;
  Darwin-arm64) target=aarch64-apple-darwin
    sha=d77ef628f094a773bf7f52d052923b910d878c0dc94e2091b7e1e9d67fd8d5e0 ;;
  Darwin-x86_64) target=x86_64-apple-darwin
    sha=60765891b22dc1f9b7d4b643f01fb1795e32c339d7a14ebd02be81e44afe3cb1 ;;
  *) echo "sieve-controls: no pinned sieve for $(uname -s)-$(uname -m)" >&2; exit 2 ;;
esac

t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
archive="sieve-${SIEVE_VERSION}-${target}.tar.gz"
curl -fsSL --retry 3 -o "$t/$archive" \
  "https://github.com/pulseengine/sieve/releases/download/${SIEVE_VERSION}/${archive}" \
  || { echo "sieve-controls: could not fetch $archive" >&2; exit 2; }
if command -v sha256sum >/dev/null; then got=$(sha256sum "$t/$archive" | cut -d' ' -f1)
else got=$(shasum -a 256 "$t/$archive" | cut -d' ' -f1); fi
[ "$got" = "$sha" ] || { echo "sieve-controls: $archive is $got, pinned $sha" >&2; exit 2; }
mkdir -p "$t/bin" && tar -xzf "$t/$archive" -C "$t/bin" ./sieve
"$t/bin/sieve" --version | head -1 | grep -qx "sieve ${SIEVE_VERSION#v}" \
  || { echo "sieve-controls: the pinned archive does not report sieve ${SIEVE_VERSION#v}" >&2; exit 2; }

cp -R "$root/examples/sieve/fixture" "$t/fixture"
(
  cd "$t/fixture"
  git init -q
  git add -A
  git -c user.name=sieve-controls -c user.email=sieve-controls@invalid \
    -c commit.gpgsign=false commit -q -m fixture
)
cd "$t/fixture"
rc=0
PATH="$t/bin:$PATH" "$plant" "$@" || rc=$?
exit "$rc"
