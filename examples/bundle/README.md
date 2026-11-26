# Evidence-bundle pack

Negative controls for a release's evidence bundle (docs/plan/v0.4.0.md,
bet C). The O2 guard reads: "The verify command must fail when any piece of
evidence is removed (negative control in CI)". No single O2 verify command
exists yet, so the pack binds to the bundle's own `verify.sh`, as step 0
decided (plant#25).

The fixture, `fixture/bundle/`, has the layout of wohl's
`wohl-v0.7.0-evidence-bundle.tar.gz`: a rivet trace, a witness MC/DC report,
a sigil (wsc) attestation, `TOOLING.txt` and `MANIFEST.sha256`. Its
`verify.sh` is a byte copy of wohl v0.7.0's; `tests/bundle.rs` fails when the
copy drifts. The contents are synthetic and small. The attestation is a
24-byte module signed with wsc 0.12.1 and a key made for this fixture.

| manifest | gate | every control must |
|---|---|---|
| `ci-controls.toml` | `bundle/verify.sh` | PASS |
| `ci-controls.stand-in.toml` | `test -s bundle/MANIFEST.sha256`, a check that the bundle exists | report FAIL-OPEN |
| `ci-controls.blind.toml` | `bundle/verify.sh` | report FAIL-OPEN |

| control | planted change | verify.sh rejects it because |
|---|---|---|
| trace-removed | both trace files deleted, and their lines dropped from the manifest | `trace/trace-export.yaml missing or empty` |
| witness-removed | the MC/DC report deleted, and its line dropped | `witness/mcdc-report.txt has no 'gap-total: N' line` |
| attestation-removed | the signed module and the key deleted, and their lines dropped | `signature verifier exited non-zero` |
| trace-altered | coverage.json changed, manifest kept | `trace/coverage.json: FAILED` (checksum) |
| witness-altered | `gap-total: 1`, manifest rewritten to match | `unresolved MC/DC gap row(s)` |
| attestation-altered | one byte of the signed module changed, manifest rewritten to match | the signature does not verify |
| trace-rewritten (blind) | coverage.json changed, manifest rewritten to match | it doesn't |

A removal drops the piece's manifest lines too, so that the piece's own check
rejects it, not the checksum step. The blind control is what `verify.sh`
cannot see. A trace altered together with its manifest is consistent inside
the bundle; `verify.sh` checks only that the trace is present and is valid
JSON. Only the outer chain rejects it: the release's cosign-signed
SHA256SUMS.txt covers the bundle tarball. `verify.sh` says so itself, and
leaves that chain to the release notes.

Run it with `scripts/proof-packs.sh target/debug/plant bundle`. It needs
`wsc` on PATH, or `$SIGIL` naming it. CI's `proof packs (bundle)` job installs
wsc 0.12.1 from sigil's release. It accepts the binary only when SHA256SUMS.txt
is signed by sigil's release workflow for that tag, and the binary matches.

Measured on 2026-11-26 with wsc 0.12.1: real gate 6 of 6 PASS, stand-in 6 of 6
FAIL-OPEN, blind 1 of 1 FAIL-OPEN.

To adopt it in a repository that ships a bundle, unpack the bundle as
`bundle/` in the fixture and point each control's plant at the real file
names. Changing wohl's verify.sh also needs the pinned sha256 in
`tests/bundle.rs` moved.

Not claimed:
- that the inner checks prove the evidence is true; they show only that it
  is present and consistent;
- that this covers bundles other than wohl's layout;
- that a verify.sh which passes the pack also checks the outer chain.
