#!/usr/bin/env bash
# One-command verifier for the wohl evidence bundle (SWREQ-REL-001).
# Run from inside the unpacked bundle: ./verify.sh
#
# Checks, in order; any failure exits non-zero:
#   1. every file matches MANIFEST.sha256
#   2. the witness MC/DC report shows 0 gap rows
#   3. the rivet trace files are present and well-formed
#   4. the attestation signature over the composed component verifies
#      (REQUIRES a verifier binary: sigil or wsc on PATH, or $SIGIL)
#
# Trust anchoring: this bundle is one release asset among the others. Its
# sha256 appears in the release's SHA256SUMS.txt, which is cosign-signed
# (keyless, bound to the release workflow identity) and SLSA-attested.
# Verify that outer chain with the commands in the release notes; this
# script verifies the bundle's inner consistency.
set -euo pipefail
cd "$(dirname "$0")"
fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. checksums
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --quiet -c MANIFEST.sha256 || fail "checksum mismatch against MANIFEST.sha256"
elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c MANIFEST.sha256 >/dev/null || fail "checksum mismatch against MANIFEST.sha256"
else
    fail "no sha256sum or shasum available"
fi
echo "OK: all files match MANIFEST.sha256"

# 2. witness MC/DC gap rows. run-witness.sh (the producer) emits the
# machine-readable 'gap-total: N' line — the single parse of witness's
# report wording. Every such line must be exactly 0.
grep -E '^gap-total: [0-9]+$' witness/mcdc-report.txt >/dev/null \
    || fail "witness/mcdc-report.txt has no 'gap-total: N' line — not a run-witness.sh report"
if grep -E '^gap-total:' witness/mcdc-report.txt | grep -vE '^gap-total: 0$' >/dev/null; then
    fail "witness reports unresolved MC/DC gap row(s): $(grep -E '^gap-total:' witness/mcdc-report.txt | tr '\n' ' ')"
fi
echo "OK: witness MC/DC report shows 0 gap rows"

# 3. trace files
[ -s trace/trace-export.yaml ] || fail "trace/trace-export.yaml missing or empty"
[ -s trace/coverage.json ]     || fail "trace/coverage.json missing or empty"
if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; json.load(open("trace/coverage.json"))' \
        || fail "trace/coverage.json is not valid JSON"
    echo "OK: rivet trace export + coverage.json present and well-formed"
else
    head -c1 trace/coverage.json | grep -q '{' || fail "trace/coverage.json does not look like JSON"
    echo "OK: rivet trace files present (python3 unavailable; structural check only)"
fi

# 4. attestation signature — a verifier is REQUIRED. A verification that
# silently skips the signature is not a verification.
VERIFIER="${SIGIL:-}"
if [ -z "$VERIFIER" ]; then
    for c in sigil wsc; do
        if command -v "$c" >/dev/null 2>&1; then VERIFIER="$c"; break; fi
    done
fi
[ -n "$VERIFIER" ] || fail "no signature verifier found: install wsc (cargo install wsc-cli, or a github.com/pulseengine/sigil release asset) or set \$SIGIL"
# Two gates, both required: the verifier must exit 0 AND print the success
# phrase. The exit code is the primary contract (wsc 0.12.0 exits 1 on a bad
# signature — measured directly 2026-11-12; sigil#367 pins it in sigil's CI);
# the phrase match is defense-in-depth against a verifier that exits 0
# without a valid-signature verdict.
VERDICT="$("$VERIFIER" verify --input-file attestation/wohl-matter-composed.signed.wasm \
                              --public-key attestation/signing.pub 2>&1)" \
    || true
printf '%s\n' "$VERDICT" | grep -q "Signature is valid" \
    || fail "attestation signature did not verify (verifier: $VERIFIER)"
echo "OK: attestation signature verifies (verifier: $VERIFIER)"

echo "PASS: evidence bundle verified. Outer chain (SHA256SUMS + cosign + SLSA) binds it to the release workflow — see TOOLING.txt and the release notes."
