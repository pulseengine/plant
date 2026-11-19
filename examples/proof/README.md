# Proof-gate packs

Planted escapes that a proof gate must reject: docs/plan/v0.4.0.md, bet A,
for the O3 indicator. The indicator asks whether a proof check, with a kernel
axiom audit and a trust-escape ledger, is a required check. A pack shows what
such a gate rejects, and that a weaker gate does not.

Each pack has a fixture with its own gate (`check.sh`) and three manifests:

| manifest | gate | every control must |
|---|---|---|
| `ci-controls.toml` | the real gate | PASS |
| `ci-controls.stand-in.toml` | a gate that only type-checks | report FAIL-OPEN |
| `ci-controls.blind.toml` | the real gate, with the hole in a file it never builds | report FAIL-OPEN |

`scripts/proof-packs.sh target/debug/plant [pack...]` runs all three manifests
and asserts those verdicts. CI runs it in the `Proof packs` workflow.

## lean

The gate is `lake build` plus `#print axioms` over the checked theorems. It
allows only `propext`, `Classical.choice` and `Quot.sound`.

| control | planted change | the gate rejects it because |
|---|---|---|
| lean-sorry | a proof replaced by `sorry` | `sorryAx` is a disallowed axiom |
| lean-axiom | `axiom cheat` used as a proof | `Fixture.cheat` is a disallowed axiom |
| lean-hole-in-unbuilt-file (blind) | `sorry` in `scratch/`, outside the library | it doesn't: the file is never built |

A plain `lake build` accepts both `sorry` (with a warning) and a new axiom.
That is why the stand-in must turn both controls FAIL-OPEN.

Measured on 2026-11-20 with Lean v4.36.0: real gate 2 of 2 PASS, stand-in 2
of 2 FAIL-OPEN, blind 1 of 1 FAIL-OPEN.

Not claimed: that the fixture's proofs are meaningful, that this audit is the
right one for any particular repository, or that passing the pack makes a
repository's proofs sound.

## kani

The gate is `cargo kani`, followed by a ledger check: every `kani::assume`
in `src/` must be listed in `assumes.ledger`. Comment lines are not code. The
O3 indicator counts input-narrowing assumptions in the ledger, and its guard
says they may only shrink, because a bug outside the assumed range is
invisible to the proof.

| control | planted change | the gate rejects it because |
|---|---|---|
| kani-narrowing-assume | a new `kani::assume(x < 50)` in the harness | it is not in `assumes.ledger` |
| kani-assume-in-comment (blind) | the same assume, only as a comment | it doesn't: a comment narrows nothing |

Plain `cargo kani` still verifies the narrowed harness, so the stand-in must
report FAIL-OPEN.

Measured on 2026-11-20 with Kani 0.67.0: real gate 1 of 1 PASS, stand-in 1
of 1 FAIL-OPEN, blind 1 of 1 FAIL-OPEN.
