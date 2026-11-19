# Example: false-skip controls for sieve

[sieve](https://github.com/pulseengine/sieve) decides, per pull request,
which heavy CI jobs a change touches. A false skip is the failure that
matters: a job a change touches is not run, and CI reports green. These
controls live in plant, apart from sieve, so the decider and its check stay
separate (interface: seam pulseengine/sieve#3).

`fixture/` is a small Cargo workspace with two sieve classes:

| class | builds | indirect inputs |
|---|---|---|
| core | fx-core | `shared/table.rs` through `#[path]`, `data/limits.txt` through `include_str!` |
| proofs | fx-proofs | all of fx-core through the path dependency |

The check is "sieve selects nothing for this change". The baseline change
lists only docs, so sieve must select no class and must not run everything.
Each plant replaces the change list (`.plant/changed.md`):

| control | planted change | sieve must |
|---|---|---|
| core-via-path-module | `shared/table.rs` | select core through the graph (`run_all=false`) |
| core-via-include-str | `data/limits.txt` | select core through the graph |
| proofs-via-path-dep | `core/src/lib.rs` | select proofs through the graph |
| proofs-own-source | `proofs/src/lib.rs` | select proofs through the graph |
| workflow-runs-all | `.github/workflows/ci.yml` | run everything, with a path reason |
| unknown-path-runs-all | `newtool/x.rs` | run everything, with a path reason |

If sieve skips a class that a planted change touches, the planted run stays
green and plant reports FAIL-OPEN. The change list is a `.md` file because
sieve's docs rule treats a markdown file as inert only when no non-markdown
file names it.

The true inputs come from cargo, not from `sieve graph`.
`tests/sieve_fixture.rs` builds each class's package and reads the dep-info of
every crate compiled. It requires every true-input plant to be an input of
its class, and the baseline and run-all plants to be inputs of none.

Run: `scripts/sieve-controls.sh target/debug/plant` fetches sieve v0.2.0 by
URL, checks it against a pinned digest (copied from its cosign-verified
SHA256SUMS.txt), copies the fixture into a throwaway git repository and runs
plant there. CI runs the same script in the `sieve false-skip controls` job.

Measured on 2026-11-19 against sieve v0.2.0 (cosign-verified with
release.yml@refs/tags/v0.2.0): 6 of 6 PASS. Against a stand-in sieve that
selects nothing for every change, all 6 reported FAIL-OPEN. The same held for
sieve v0.1.0 on 2026-11-18.
