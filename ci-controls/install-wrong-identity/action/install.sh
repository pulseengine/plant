#!/usr/bin/env bash
# Installs a released plant for the GitHub Action, verified before it runs:
#   1. SHA256SUMS.txt is signed by plant's own release workflow for exactly
#      this tag (cosign, keyless, exact certificate identity);
#   2. the archive's sha256 equals its line in those signed sums;
#   3. the binary reports `plant VERSION`.
# Any failure exits 1 and installs nothing. Needs curl, cosign, sha256sum
# (or shasum) and tar.
#
#   action/install.sh VERSION DEST     e.g. action/install.sh 0.5.0 "$RUNNER_TEMP/plant-bin"
#
# PLANT_RELEASES overrides the download base (tests); the signer identity
# cannot be overridden.
set -euo pipefail

version=${1:?usage: install.sh VERSION DEST}
dest=${2:?usage: install.sh VERSION DEST}
case $version in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "plant install: version $version is not X.Y.Z" >&2; exit 2 ;;
esac

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *) echo "plant install: no release archive for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

tag="v$version"
archive="plant-$tag-$target.tar.gz"
base=${PLANT_RELEASES:-https://github.com/pulseengine/plant/releases/download}
identity="https://github.com/pulseengine/sieve/.github/workflows/release.yml@refs/tags/$tag"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for f in SHA256SUMS.txt SHA256SUMS.txt.cosign.bundle "$archive"; do
  curl -fsSL --retry 3 -o "$work/$f" "$base/$tag/$f" ||
    { echo "plant install: cannot download $f for $tag" >&2; exit 1; }
done

if ! cosign verify-blob \
  --certificate-identity "$identity" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --bundle "$work/SHA256SUMS.txt.cosign.bundle" "$work/SHA256SUMS.txt" >/dev/null 2>"$work/cosign.err"; then
  cat "$work/cosign.err" >&2
  echo "plant install: cosign rejected SHA256SUMS.txt for $tag (identity $identity)" >&2
  exit 1
fi

want=$(awk -v a="./$archive" -v b="$archive" '$2 == a || $2 == b { print $1 }' "$work/SHA256SUMS.txt")
if command -v sha256sum >/dev/null; then
  have=$(sha256sum "$work/$archive" | cut -d' ' -f1)
else
  have=$(shasum -a 256 "$work/$archive" | cut -d' ' -f1)
fi
if [ -z "$want" ] || [ "$want" != "$have" ]; then
  echo "plant install: sha256 mismatch for $archive: signed sums say ${want:-nothing}, archive is $have" >&2
  exit 1
fi

mkdir -p "$work/x" "$dest/bin"
tar -xzf "$work/$archive" -C "$work/x"
bin=$(find "$work/x" -type f -name plant -perm -u+x | head -1)
[ -n "$bin" ] || { echo "plant install: no plant binary in $archive" >&2; exit 1; }
got=$("$bin" --version)
if [ "$got" != "plant $version" ]; then
  echo "plant install: $archive reports \"$got\", not \"plant $version\"" >&2
  exit 1
fi
cp "$bin" "$dest/bin/plant"
echo "plant install: plant $version for $target, cosign identity $identity, sha256 $have"
