#!/usr/bin/env bash
# Runs examples/sieve's false-skip controls against the pinned sieve release.
#
#   scripts/sieve-controls.sh PLANT_BIN
#
# The archive is fetched by URL and checked against a digest pinned here. The
# digests are copied from sieve v0.1.0's SHA256SUMS.txt, which was verified
# with cosign against
# https://github.com/pulseengine/sieve/.github/workflows/release.yml@refs/tags/v0.1.0
# when this pin was set. Moving the pin is a reviewed change to this file.
# The fixture is copied into a throwaway git repository, since plant runs on
# HEAD, and plant runs there with this sieve first on PATH.
#
# Exit: plant's own exit code; 2 if sieve cannot be fetched or does not match.
set -euo pipefail

SIEVE_VERSION=v0.1.0

plant=$1; shift
case $plant in /*) ;; *) plant="$PWD/$plant" ;; esac
root=$(cd "$(dirname "$0")/.." && pwd)

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl
    sha=9d64d340fbc17ac6f97500fa3e91ab065355d6a7563815eff2b26e629b228e39 ;;
  Linux-aarch64) target=aarch64-unknown-linux-musl
    sha=b5420adbf0cb310a520d4f4aa94bfe89ecc0ab99b7a3a6bc18162f91ddd0c982 ;;
  Darwin-arm64) target=aarch64-apple-darwin
    sha=ba6863b40fa4407ebc8edc5ec4418e80d6b6f423fdfb91b38e062ab3ef08d8a4 ;;
  Darwin-x86_64) target=x86_64-apple-darwin
    sha=e9de0c82dc0433419e527b9bf667aa87a524748d3a582eb820ccd2bde18e04a0 ;;
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
