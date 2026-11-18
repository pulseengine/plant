# Example: sigil's fork-guard check as plant controls

sigil's `verification/selfhosted-guard/check.py` proves its own rules with an
embedded `--self-test`: synthetic workflows in a temp dir. This manifest turns
those cases into plant controls that run on sigil's real tree instead:

- the baseline runs the guard on sigil's own `.github/workflows` and must be
  green;
- each plant adds one workflow, `.github/workflows/planted.yml`, that the guard
  must reject, and the regex requires the rejection for that job, so a YAML
  error or a crash does not count;
- `compared-nothing` deletes `.github/workflows`, and the guard must refuse to
  pass on nothing.

To use it, copy `ci-controls.toml` and `ci-controls/` to the root of sigil and
run `plant`. This is an example in plant's repository; sigil has not adopted it.

Since sigil#388 (8df6dcc) the guard that gates sigil's CI is the WASI
component in `verification/selfhosted-guard`, and `check.py` is its
equivalence oracle. The 13 controls build the component for `wasm32-wasip2`
(the toolchain comes from sigil's rust-toolchain.toml) and run it under
wasmtime, as sigil's "No fork PR reaches a self-hosted runner" job does.
`bare-label-check-py` runs `check.py` on the same plant as a cross-check. Set
`CARGO_TARGET_DIR` to a dedicated absolute directory so the scratch copies
share one build: the plants change only workflow files, never the guard's
source. `wasmtime` must be on PATH as a real binary. A varve shim refuses to
run in plant's scratch copies, because sigil pins no varve layer.

Last run against sigil `main` at 8df6dcc on 2026-11-18, with the released
plant v0.2.0 and wasmtime 50.0.1 (varve layer 2026.11.10): 14 of 14 controls
PASS. Earlier runs on check.py: 13 of 13 at c18be87, 12 of 12 at f51d09b.
While the example was written, four plants first went red on a YAML quoting
error in the plant itself; the regex reported them as FAIL ("went red for
some other reason") instead of letting them pass.
