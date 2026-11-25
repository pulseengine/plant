# Lockfile pack

Planted bad locks that a lockfile gate must reject, for repositories that
take plant's controls into their CI (the shared CI baseline pilot). The pack
shows what plant's own lockfile gate rejects, and that a weaker gate does not.

The fixture has a root Cargo workspace and a separate nested workspace,
`tools/`, with its own `Cargo.lock`. Its gate, `check.sh`, is a byte-for-byte
copy of plant's `scripts/lockfiles.sh`: every Cargo workspace in the tree has a
committed lock, `cargo audit --deny warnings` reads it clean, and
`cargo metadata --locked` would not change it. Workspaces are found from the
`Cargo.toml` files, so the nested one is checked without being named.
`tests/lockfiles.rs` fails when the copy drifts.

| manifest | gate | every control must |
|---|---|---|
| `ci-controls.toml` | the real gate | PASS |
| `ci-controls.stand-in.toml` | `cargo audit` and `cargo metadata --locked` on the root lock only; and, for the vulnerable lock, a binding check alone | report FAIL-OPEN |
| `ci-controls.blind.toml` | the real gate, with the bad lock under `target/` | report FAIL-OPEN |

| control | planted change | the real gate rejects it because |
|---|---|---|
| lock-stale-nested | `tools/gen` bumped to 0.0.1, lock left at 0.0.0 | `tools/Cargo.lock` is not binding |
| lock-missing-nested | `tools/Cargo.lock` deleted | the nested workspace has no lock |
| lock-vulnerable-nested | `tools/gen` depends on `time` =0.1.43, and the lock resolves it | cargo audit reports RUSTSEC-2020-0071 |
| lock-vulnerable-in-target (blind) | the same lock under `target/vendor/` | it doesn't: build output is not a workspace |

Planted lock and manifest files are stored as `<name>.planted`, which plant
writes into the scratch copy as `<name>`. Dependency bots read `Cargo.lock`
and `Cargo.toml` anywhere in a tree; under their real names the deliberately
vulnerable lock would draw an alert and a PR that removes the plant.

Run it with `scripts/proof-packs.sh target/debug/plant locks` (needs
cargo-audit). plant's required `lockfiles` CI job runs it.

To adopt it, copy `check.sh` and the controls whose shape matches your tree,
then point each control's plant at your own nested workspace. The stand-in
manifest is the negative control: if your current gate passes it, that gate
reads only the root lock.

The vulnerable lock is binding, so only an audit can reject it: a gate that
checks freshness first must not catch it for the wrong reason (plant#43,
found in jig's adoption run). Resolving it under `--locked` needs the
crates.io index, so that control needs network.

Measured on 2026-11-25 with cargo-audit 0.22.1: real gate 3 of 3 PASS,
stand-in 4 of 4 FAIL-OPEN, blind 1 of 1 FAIL-OPEN.

Not claimed: that this gate suits every repository, that it covers lockfile
ecosystems other than Cargo, or that a clean audit means the dependencies are
safe; cargo audit knows only the advisories in the RustSec database.
