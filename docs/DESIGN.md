# plant design (from the Python prototype, 2026-11-14)

## Manifest schema

`ci-controls.toml` at the repository root, an array of `[[control]]` tables.

| key | type | required | meaning |
|---|---|---|---|
| `id` | string | yes | unique within the manifest |
| `check` | string | yes | the CI check name it protects (reporting only) |
| `run` | string | yes | shell command, run from the scratch root |
| `plant` | string | yes | directory relative to the repo root, must not escape it |
| `expect` | string | no | only `"fail"` is accepted (default) |
| `stderr_or_stdout_matches` | string | no | regex over combined output of the planted run |
| `timeout` | number | no | seconds per run, default 300 |
| `baseline` | bool | no | default true: run unplanted first and require exit 0 |
| `[control.env]` | table of strings | no | extra environment for both runs |

Unknown keys, duplicate ids, a bad regex or a wrong type: exit 2.

## Verdicts

Exit 0 all controls pass; 1 any control failed or zero controls; 2 bad manifest,
git missing, or no commit.

A control FAILS when:
- the planted run exits 0 (fail-open);
- the baseline is red;
- either run times out (a hang is not a red);
- the command cannot start (exit 126/127 is not a red);
- the regex does not match;
- the plant directory is missing, or has no files and no deletes (a `delete`
  file with only comments counts as empty);
- a `delete` entry escapes the root or names a path not in the scratch copy.

Output: one `PASS`/`FAIL  id  [check]: reason` line per control, the last lines
of output for failures, a summary. `--format json`, `--only ID`.

## Mechanics

- Scratch copy: export `HEAD` (git archive) once, copy it per control; the
  working tree is never touched.
- Baseline runs in its own copy; the copy is reset before the plant is applied,
  because a check may write caches or reports.
- The plant is read from the working tree, not `HEAD`, so a new control can be
  tried before it is committed. Plant files overlay at the same relative path.
  A file named `delete` in the plant directory lists paths to remove, one per
  line, `#` comments allowed.
- Note on stderr when tracked files are dirty: edits to the checks themselves
  are then not under test.
- Timeouts kill the whole process group, so children of the shell die too.

## Pitfalls found

- The scratch copy has no `.git`; checks that shell out to git need extra work
  (e.g. an option to keep a shallow `.git`).
- Exit 126/127 must not read as a red.
- `baseline = false` must be reported as skipped, not silently passed.

## Self-test cases (both directions)

Good check fails on the plant: PASS; with matching regex: PASS; regex mismatch:
FAIL; fail-open: FAIL; plant the check ignores: FAIL; red baseline: FAIL;
missing / empty / comment-only-delete plant: FAIL; timeout: FAIL; delete-only
plant removing a required file: PASS; escaping delete path: FAIL; command not
found: FAIL; `baseline = false`: PASS reported as skipped; `[control.env]`
reaches the command; overall exit 1 when any control failed; working tree
byte-identical afterwards and no leaked files; temp dir cleaned; all-green
manifest exits 0; `--only` valid id runs one, unknown id exits 2; empty
manifest exits 1; unparseable / missing / duplicate-id manifest and
`expect = "pass"` exit 2; directory with no git history exits 2.

## First adopters

The hand-written fork-guard self-tests in gale, ordeal, sigil and relay, and
the gate fixes from the 13 November sweep (witness#373, scry#240,
rules_rocq_rust#58, ordeal#319, meld#610), each ship a planted input that can
become a manifest entry.

## Prior art

No established tool found (2026-11-14). Closest: one small repo with the same
idea (ci-gate-forgery-test), blog write-ups of injected-defect harnesses, and
arXiv 2509.14347 on silent CI failures, which proposes automated detection as
future work. actionlint/zizmor check workflow syntax and security, and
mutation tools (cargo-mutants, PIT, mutmut) test test suites, not gates.
